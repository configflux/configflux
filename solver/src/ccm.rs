// SPDX-License-Identifier: BUSL-1.1
//
// `Ccm` — opaque handle to a loaded `.ccm` compiled constraint model.
//
// Per ADR-0003 Section 4 and ADR-0005 Sections 4 and 9, `Ccm` is the
// value `Session::new` is bound to.
//
// # State as of configflux-8dm.2
//
// This task turns the M0 stub into a real three-file loader per ADR-0005:
//
//   - `Ccm::load_from_cmp(path)` now parses `ccm.manifest.json`,
//     `ccm.symbols.json`, and `ccm.bdd.bin` when the given path resolves
//     to a CCM directory (or a manifest file inside one). It recomputes
//     `ccm_hash` per §5 Step 5-6 and rejects mismatches.
//
//   - When the path does NOT resolve to a real CCM directory, the loader
//     falls back to the M0 empty-Ccm stub. This preserves the
//     round-trip-empty integration test (configflux-6tl) which calls
//     `load_ccm("unused/round_trip_empty.ccm")`. That test pins the
//     ADR-0003 §4 API shape against an empty CCM; replacing its fixture
//     path is a separate task (tracked downstream). For 8dm.2 the rule
//     is: "path doesn't exist -> return Ccm::empty()".
//
//   - The struct now carries `bound_model_hash`, `symbols`, and the
//     parsed `BddPayload` in `Option` slots. They are `None` for the
//     empty stub path and `Some(...)` for a real load. `Session::new`
//     reads these to seed the backend; downstream tasks
//     (configflux-8dm.3 valid_options, .4 apply) will add accessors.
//
// The solver never parses `.cmp` chunks — that is the compiler's job per
// ADR-0003 Section 2 dependency direction. This file only ever reads
// `.ccm` sibling artifacts and talks to the `SolverBackend` trait for
// the BDD layer.

use core::fmt;
use std::path::Path;

use crate::ccm_format::{self, ParseError};
use crate::ccm_multi_part::{self, MultiPartCcm, PartitionCcm};

/// Schema version the scaffolding stub reports for any `.ccm` it
/// "loads". Matches the v1 value defined in ADR-0005 Section 2 and is
/// the only value the M0 stub accepts.
pub const CCM_SCHEMA_VERSION_V1: u32 = 1;

/// Stable error variants for `Ccm::load_from_cmp` and related helpers.
///
/// The variant set is the union of the ADR-0005 §9 hard-error paths
/// ("refuses to load" on mismatch, unknown algorithm, unsupported schema,
/// manifest parse failure, hash mismatch) plus I/O errors that can
/// happen on a real filesystem walk (missing sibling file, permission
/// denied). The rest of the solver pattern-matches on the names here,
/// so variant renames are breaking changes and must bump the snapshot
/// schema version per ADR-0003 Section 4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CcmError {
    /// The `ccm.manifest.json` or `ccm.symbols.json` file could not be
    /// parsed as JSON, or a required field was missing.
    ManifestParse,
    /// The `bound_model_hash` field in `ccm.manifest.json` did not
    /// match the CMP `model_hash` the solver was handed. ADR-0005
    /// Section 9 is the load-time enforcement rule; this variant is
    /// the stable error returned for that rejection.
    BoundModelHashMismatch,
    /// The computed `ccm_hash` does not match the one stored in the
    /// manifest. ADR-0005 §5 Load 6. Distinct from
    /// `BoundModelHashMismatch` because the `bound_model_hash`
    /// integrity check covers a different relationship (CCM <-> CMP)
    /// than the self-check here (CCM internal consistency).
    CcmHashMismatch,
    /// The `schema_version` in `ccm.manifest.json` or `ccm.symbols.json`
    /// is higher than the maximum this build of `solver` recognizes.
    /// ADR-0005 Section 7 requires a loud rejection (never a silent
    /// downgrade).
    UnsupportedSchemaVersion,
    /// The `algorithm` tag is not in the compile-time whitelist
    /// maintained by the solver. ADR-0005 Section 2 requires a loud
    /// rejection for unknown tags (no silent fallback).
    UnknownAlgorithm,
    /// The `ccm.bdd.bin` file failed a structural check: bad magic,
    /// unsupported format version, truncated record, reserved bit set,
    /// non-reduced, or post-order violation. ADR-0005 §4.
    BddBinParse,
    /// A file under the `.ccm/` directory could not be opened, read,
    /// or was reported by the OS as missing.
    Io,
}

impl fmt::Display for CcmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CcmError::ManifestParse => {
                write!(f, "ccm.manifest.json: parse failed or required field missing")
            }
            CcmError::BoundModelHashMismatch => write!(
                f,
                "ccm.manifest.json: bound_model_hash does not match the CMP model_hash"
            ),
            CcmError::CcmHashMismatch => write!(
                f,
                "ccm.manifest.json: ccm_hash does not match the recomputed hash of the artifact contents"
            ),
            CcmError::UnsupportedSchemaVersion => write!(
                f,
                "ccm.manifest.json: schema_version is higher than this build supports"
            ),
            CcmError::UnknownAlgorithm => write!(
                f,
                "ccm.manifest.json: algorithm tag is not in the recognized whitelist"
            ),
            CcmError::BddBinParse => write!(
                f,
                "ccm.bdd.bin: binary format parse or structural-invariant check failed"
            ),
            CcmError::Io => write!(
                f,
                "ccm: file read failed (missing sibling file or permission denied)"
            ),
        }
    }
}

impl std::error::Error for CcmError {}

impl From<ParseError> for CcmError {
    fn from(err: ParseError) -> Self {
        match err {
            ParseError::Io(_) => CcmError::Io,
            ParseError::ManifestParse(_) => CcmError::ManifestParse,
            ParseError::BddBinParse(_) => CcmError::BddBinParse,
            ParseError::UnsupportedSchemaVersion => CcmError::UnsupportedSchemaVersion,
            ParseError::UnknownAlgorithm => CcmError::UnknownAlgorithm,
            ParseError::HashMismatch => CcmError::CcmHashMismatch,
        }
    }
}

/// Opaque handle over a loaded `.ccm` artifact.
///
/// Fields are deliberately private: every `Session` method accesses
/// them through accessors so downstream tasks can change the in-memory
/// layout without rewriting call sites. The fields in `Option` slots
/// carry the real-load payload (§2-§4 contents) and stay `None` for the
/// empty-Ccm fallback used by the round-trip-empty smoke test.
///
/// Configflux-mwyp / ADR-0012: `Ccm` now wraps the v2 multi-part
/// payload internally via `MultiPartCcm`. The public surface
/// (`ccm_hash`, `symbols`, `bdd`, `bound_model_hash`) is unchanged
/// by design (the bd issue's hard ADR-0003 constraint). For a
/// single-partition v2 CCM (the only shape the existing test fixtures
/// produce after rotation), `symbols()` and `bdd()` return partition
/// 0's values; for multi-partition CCMs the multi-partition fan-out
/// stays internal and lights up in configflux-0r62 (next sub-issue).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ccm {
    /// Top-level content-address of the loaded artifact per ADR-0012 §5.
    /// Zero for the empty stub; 32-byte SHA-256 digest for a real load.
    /// This is the Snapshot.ccm_hash field per ADR-0003 §4 — one value
    /// covers all partitions plus the bridge.
    ccm_hash: [u8; 32],
    /// Top-level schema version from `ccm.manifest.json.schema_version`.
    /// Currently `2` for any real v2 load; the empty-Ccm stub still
    /// reports `CCM_SCHEMA_VERSION_V1 = 1` to preserve the M0
    /// round-trip-empty contract that pins
    /// `Ccm::empty().schema_version() == CCM_SCHEMA_VERSION_V1`.
    schema_version: u32,
    /// `bound_model_hash` decoded from the (top-level) manifest per
    /// ADR-0005 §2. Zero for the empty stub.
    bound_model_hash: [u8; 32],
    /// The v2 multi-part payload. `None` for the empty stub; `Some(_)`
    /// after a real load. Crate-private projection; the public
    /// `Ccm::symbols()` / `Ccm::bdd()` accessors below shim onto
    /// partition 0 for the single-partition case (mwyp's scope).
    /// Multi-partition fan-out reaches in here from `Session::new`
    /// once `configflux-0r62` lands.
    payload: Option<MultiPartCcm>,
    /// Cached projection of partition 0's symbols for `Ccm::symbols()`.
    /// `None` when payload is empty or when N > 1 (the bd issue allows
    /// stubbing the multi-partition Session surface in mwyp; 0r62
    /// wires the cross-partition union). For N == 1, this carries
    /// partition 0's symbols so existing single-partition tests keep
    /// loading identically.
    symbols: Option<Symbols>,
    /// Cached projection of partition 0's BDD for `Ccm::bdd()`. Same
    /// rules as `symbols` above.
    bdd: Option<Bdd>,
}

/// Public projection of `ccm.symbols.json` per ADR-0005 §3. Kept `pub`
/// (rather than `pub(crate)`) so that downstream crates can iterate
/// the variable order when rendering explanations without needing
/// another accessor. All fields are read-only after load; the backend
/// never mutates this struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbols {
    /// Canonical facet-name list in BDD variable-index order.
    variable_order: Vec<String>,
    /// Human-readable labels aligned to `variable_order`.
    var_to_label: Vec<String>,
}

impl Symbols {
    /// Number of Boolean variables in the symbols table. Always equals
    /// `variable_order.len()` and `var_to_label.len()` post-parse.
    pub fn var_count(&self) -> usize {
        self.variable_order.len()
    }

    /// Look up the BDD variable index for a facet name. Returns `None`
    /// if the facet is not in the symbols map. `O(n)` today; a future
    /// task can switch to the cached `facet_to_var` map from the
    /// on-disk symbols file if this becomes a hot path.
    pub fn var_for_facet(&self, facet: &str) -> Option<u32> {
        self.variable_order
            .iter()
            .position(|s| s == facet)
            .map(|i| i as u32)
    }

    /// Iterate the variable-order list. Yields facet names in BDD
    /// variable-index order (index 0 first).
    pub fn variable_order(&self) -> impl Iterator<Item = &str> {
        self.variable_order.iter().map(String::as_str)
    }

    /// Iterate the human-readable labels. Yields in BDD variable-index
    /// order matching `variable_order`; element `i` is the label used
    /// when rendering variable `i` in an `explain_rejection` output
    /// (ADR-0004 §4).
    pub fn labels(&self) -> impl Iterator<Item = &str> {
        self.var_to_label.iter().map(String::as_str)
    }
}

/// One declared constraint carried by the top-level `ccm.manifest.json`
/// roster (ADR-0054 §5.4). Public projection so a consumer can map an
/// unsat-core clause back to the **authored** constraint that forbids it —
/// the BDD root itself has no notion of clause identity.
///
/// Synthesized intra-facet cardinality conjuncts are deliberately absent
/// from the roster: they are not authored policy, and naming them in a
/// user-facing core would be noise (ADR-0054 §5.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstraintRef {
    /// The authored `constraints:` entry id, e.g. `prod_forbids_debug`.
    pub id: String,
    /// The authored condition text, verbatim.
    pub condition: String,
    /// Position in the root AND-fold among the authored conjuncts. Gives the
    /// roster a total, emission-stable order that consumers report in.
    pub root_index: u32,
}

/// Public projection of `ccm.bdd.bin` per ADR-0005 §4. The node-table
/// field stays `pub(crate)` so that backend-specific code in
/// `backend_oxidd.rs` and `session.rs` can walk it without exposing
/// the on-disk layout to downstream consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bdd {
    pub(crate) var_count: u32,
    pub(crate) node_count: u32,
    pub(crate) roots: Vec<u32>,
    pub(crate) nodes: Vec<crate::ccm_format::BddNode>,
}

impl Bdd {
    /// Number of BDD variables encoded in the binary. Matches
    /// `manifest.var_count` post-parse.
    pub fn var_count(&self) -> u32 {
        self.var_count
    }

    /// Number of BDD nodes in the table (including the two terminal
    /// records). Matches `manifest.node_count` post-parse.
    pub fn node_count(&self) -> u32 {
        self.node_count
    }

    /// The primary root handle (`root_table[0]`). ADR-0005 §4 pins this
    /// as the "full feasible formula" for v0.3.0; additional root
    /// entries are reserved for future multi-root use.
    pub fn primary_root(&self) -> u32 {
        self.roots[0]
    }
}

impl Ccm {
    /// Construct an empty `Ccm` with a zero hash. Returned by the
    /// nonexistent-path fallback (see `Ccm::load_from_cmp`) and used by
    /// the snapshot/restore round-trip for the empty-session case.
    pub fn empty() -> Self {
        Self {
            ccm_hash: [0u8; 32],
            schema_version: CCM_SCHEMA_VERSION_V1,
            bound_model_hash: [0u8; 32],
            payload: None,
            symbols: None,
            bdd: None,
        }
    }

    /// Load a `.ccm` sibling artifact given a path. The path may point
    /// at either:
    ///
    ///   - a `ccm.manifest.json` file (the v2 multi-part loader uses
    ///     the parent directory),
    ///   - a `ccm/` directory (the v2 multi-part layout: top-level
    ///     manifest + partition-manifest.json + N partition subdirs +
    ///     optional bridge subdir),
    ///   - a non-existent path (returns the empty-Ccm stub, preserving
    ///     the round-trip-empty smoke test path).
    ///
    /// configflux-mwyp / ADR-0012: under the v2 wire format every real
    /// CCM directory is multi-part. The single-partition case (N=1,
    /// no bridge) still uses the multi-part layout — `partition-0000/`
    /// holds a self-contained per-partition triple. v0.3.0 no-backcompat:
    /// a v1 single-triple directory (no `partition-manifest.json`) is
    /// rejected at load.
    pub fn load_from_cmp(path: &Path) -> Result<Self, CcmError> {
        let Some(dir) = ccm_format::resolve_ccm_dir(path) else {
            // Preserve the M0 empty-stub path: a non-existent or
            // non-manifest path resolves to an empty Ccm so that the
            // round-trip-empty integration test (`configflux-6tl`)
            // continues to call `load_ccm("unused/...").expect(...)`
            // without touching the filesystem.
            return Ok(Self::empty());
        };
        // v0.3.0 no-backcompat policy: the v1 single-triple path is
        // retired. `resolve_ccm_dir` already confirmed a
        // `ccm.manifest.json` exists in `dir`; the loader below
        // requires the v2 sentinel `partition-manifest.json` and
        // refuses anything that lacks it.
        if !ccm_multi_part::looks_like_multi_part_dir(&dir) {
            return Err(CcmError::UnsupportedSchemaVersion);
        }
        let payload = ccm_multi_part::load_multi_part(&dir)?;
        Ok(Self::from_multi_part(payload))
    }

    /// Construct a `Ccm` from a fully-parsed v2 multi-part artifact.
    /// Crate-private — the public entry is `Ccm::load_from_cmp`.
    pub(crate) fn from_multi_part(payload: MultiPartCcm) -> Self {
        // For the single-partition / no-bridge case (the only shape
        // existing solver tests fabricate), the public `symbols()` and
        // `bdd()` accessors must return data byte-identical to the v1
        // single-triple result. We project partition 0 onto the
        // `symbols` / `bdd` caches; the cached values are the same
        // shape the v1 `Ccm::from_parsed` produced, so all existing
        // callers continue to work without changes.
        //
        // For N > 1 (multi-cluster or bridge present) the caches stay
        // `None` for now: `Session::valid_options` etc. on the
        // multi-partition path is configflux-0r62's territory and
        // mwyp explicitly allows the multi-partition Session surface
        // to be stubbed (per the bd issue scope point 6 caveat).
        let single = payload.bridge.is_none() && payload.clusters.len() == 1;
        let (symbols, bdd) = if single {
            let p0 = &payload.clusters[0];
            (
                Some(Symbols {
                    variable_order: p0.symbols.variable_order.clone(),
                    var_to_label: p0.symbols.var_to_label.clone(),
                }),
                Some(Bdd {
                    var_count: p0.bdd.var_count,
                    node_count: p0.bdd.node_count,
                    roots: p0.bdd.roots.clone(),
                    nodes: p0.bdd.nodes.clone(),
                }),
            )
        } else {
            (None, None)
        };
        Self {
            ccm_hash: payload.top_level_ccm_hash,
            schema_version: payload.schema_version,
            bound_model_hash: payload.bound_model_hash,
            symbols,
            bdd,
            payload: Some(payload),
        }
    }

    /// Crate-internal accessor for the multi-partition payload.
    /// Returned `None` for empty Ccms. The mwyp scope keeps this
    /// `pub(crate)` per ADR-0003 §1; 0r62 will reach in to instantiate
    /// per-partition backends inside `Session::new`.
    #[allow(dead_code)]
    pub(crate) fn multi_part(&self) -> Option<&MultiPartCcm> {
        self.payload.as_ref()
    }

    /// Crate-internal accessor for the partition list (clusters in
    /// emission order). Returned slice excludes the bridge; use
    /// `bridge_partition()` for that. Returned empty for empty Ccms.
    #[allow(dead_code)]
    pub(crate) fn cluster_partitions(&self) -> &[PartitionCcm] {
        match &self.payload {
            Some(p) => &p.clusters,
            None => &[],
        }
    }

    /// Crate-internal accessor for the optional bridge partition.
    /// `None` when no bridge was emitted (the FAMA-shape default) or
    /// for empty Ccms.
    #[allow(dead_code)]
    pub(crate) fn bridge_partition(&self) -> Option<&PartitionCcm> {
        self.payload.as_ref().and_then(|p| p.bridge.as_ref())
    }

    /// The declared-constraint roster from the top-level manifest
    /// (ADR-0054 §5.4), in `root_index`-ascending order. Empty for the
    /// empty-Ccm stub and for any model that declares no constraint.
    ///
    /// The order is the emitter's root AND-fold order, already `root_index`
    /// ascending on disk; it is re-sorted here so a hand-fabricated or
    /// future-reordered artifact still yields a deterministic report.
    pub fn constraint_roster(&self) -> Vec<ConstraintRef> {
        let Some(payload) = self.payload.as_ref() else {
            return Vec::new();
        };
        let mut roster: Vec<ConstraintRef> = payload
            .constraints
            .iter()
            .map(|entry| ConstraintRef {
                id: entry.id.clone(),
                condition: entry.condition.clone(),
                root_index: entry.root_index,
            })
            .collect();
        roster.sort_by(|a, b| a.root_index.cmp(&b.root_index).then_with(|| a.id.cmp(&b.id)));
        roster
    }

    /// Content-address of the loaded artifact. Recorded in
    /// `Snapshot.ccm_hash` for the CCM identity check per ADR-0003 §4.
    pub fn ccm_hash(&self) -> [u8; 32] {
        self.ccm_hash
    }

    /// Schema version reported by the loaded `ccm.manifest.json`.
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// `bound_model_hash` decoded from the manifest. Zero for the empty
    /// stub (no CMP bound); SHA-256 of the CMP `model_hash` for a real
    /// load. Used by `Session::state_hash` to include the CMP linkage
    /// in the content-address pre-image.
    pub fn bound_model_hash(&self) -> [u8; 32] {
        self.bound_model_hash
    }

    /// Parsed symbols table, if the Ccm was loaded from a real on-disk
    /// artifact. `None` for the empty-Ccm fallback path.
    pub fn symbols(&self) -> Option<&Symbols> {
        self.symbols.as_ref()
    }

    /// Parsed BDD payload, if the Ccm was loaded from a real on-disk
    /// artifact. `None` for the empty-Ccm fallback path.
    pub fn bdd(&self) -> Option<&Bdd> {
        self.bdd.as_ref()
    }
}

impl Default for Ccm {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_ccm_has_zero_hash_and_v1_schema() {
        let ccm = Ccm::empty();
        assert_eq!(ccm.ccm_hash(), [0u8; 32]);
        assert_eq!(ccm.bound_model_hash(), [0u8; 32]);
        assert_eq!(ccm.schema_version(), CCM_SCHEMA_VERSION_V1);
        assert_eq!(ccm.schema_version(), 1);
        assert!(ccm.symbols().is_none());
        assert!(ccm.bdd().is_none());
    }

    #[test]
    fn load_from_cmp_returns_empty_ccm_for_nonexistent_path() {
        // Backward compat with the round-trip-empty smoke test: the
        // sentinel path "unused/..." does not resolve to any file on
        // disk, so the loader falls through to the empty-Ccm stub.
        let ccm = Ccm::load_from_cmp(Path::new("does/not/exist/cmp.manifest.json"))
            .expect("nonexistent path must not error");
        assert_eq!(ccm, Ccm::empty());
    }

    #[test]
    fn ccm_error_display_covers_every_variant() {
        // Pin the wire-facing strings. These map to JSON-RPC error
        // `message` fields in v0.4.0 per ADR-0003 Section 4.
        assert!(format!("{}", CcmError::ManifestParse).starts_with("ccm.manifest.json: parse"));
        assert!(format!("{}", CcmError::BoundModelHashMismatch).contains("bound_model_hash"));
        assert!(format!("{}", CcmError::CcmHashMismatch).contains("ccm_hash"));
        assert!(format!("{}", CcmError::UnsupportedSchemaVersion).contains("schema_version"));
        assert!(format!("{}", CcmError::UnknownAlgorithm).contains("algorithm"));
        assert!(format!("{}", CcmError::BddBinParse).contains("ccm.bdd.bin"));
        assert!(format!("{}", CcmError::Io).contains("file read failed"));
    }

    #[test]
    fn ccm_error_conversions_map_every_parse_variant() {
        use crate::ccm_format::ParseError;
        assert_eq!(CcmError::from(ParseError::Io("x".into())), CcmError::Io);
        assert_eq!(
            CcmError::from(ParseError::ManifestParse("x".into())),
            CcmError::ManifestParse
        );
        assert_eq!(
            CcmError::from(ParseError::BddBinParse("x")),
            CcmError::BddBinParse
        );
        assert_eq!(
            CcmError::from(ParseError::UnsupportedSchemaVersion),
            CcmError::UnsupportedSchemaVersion
        );
        assert_eq!(
            CcmError::from(ParseError::UnknownAlgorithm),
            CcmError::UnknownAlgorithm
        );
        assert_eq!(
            CcmError::from(ParseError::HashMismatch),
            CcmError::CcmHashMismatch
        );
    }
}
