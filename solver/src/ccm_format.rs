// SPDX-License-Identifier: BUSL-1.1
//
// `ccm_format` — readers for the three on-disk CCM files defined in
// ADR-0005.
//
// This module is **the only file in the solver crate** that knows the
// on-disk layout of the `.ccm` directory: `ccm.manifest.json` (§2),
// `ccm.symbols.json` (§3), and `ccm.bdd.bin` (§4). It owns the parse
// path end-to-end: canonical JSON for the two manifests, fixed-width
// little-endian records for the BDD, plus the §5 `ccm_hash` pre-image
// assembly. The output is a set of plain-Rust values (`Manifest`,
// `Symbols`, `Vec<BddNode>`) that the rest of the solver can consume
// without touching a single `std::io` call.
//
// The file is intentionally separate from `ccm.rs`: `Ccm` holds the
// parsed identity (hash + schema + handles) and is what the public API
// returns; `ccm_format` is the private parser plumbing that fills it
// in. This separation keeps the public `Ccm` API small and lets M1's
// downstream tasks (configflux-8dm.3 `valid_options`) grow their own
// module without bloating the format reader.
//
// # Scope of configflux-8dm.2
//
// This task reads:
//   - `ccm.manifest.json` — all required v1 fields, with strict
//     schema_version rejection (§7) and strict algorithm whitelist
//     rejection (§2).
//   - `ccm.symbols.json` — variable_order + facet_to_var + var_to_label,
//     with the internal-consistency check (§3 "the solver verifies at
//     load time that `facet_to_var[variable_order[i]] == i`").
//   - `ccm.bdd.bin` — magic + version + var_count + node_count +
//     root_count + root_table + node_table, with reducedness-invariant
//     structural checks (§4 "no node has low_id == high_id; no two
//     distinct nodes share the same (var_index, low_id, high_id, flags)
//     triple").
//
// It does NOT yet:
//   - Enforce canonical-JSON round-trip byte equality (§6 "re-serialize
//     the struct and compare bytes"). That is a later hardening task;
//     for 8dm.2 we use serde_json's default parser which accepts
//     whitespace the canonical format forbids. A later task bumps this
//     to the strict variant.
//   - Locate a sibling `cmp/` directory and cross-check CMP `model_hash`
//     against `bound_model_hash` (§9 Load B-D). That is the integration
//     path owned by configflux-8dm.5 (the compiler emitter) plus a
//     follow-up solver task; for 8dm.2 the caller hands us the
//     bound_model_hash and we store it without CMP cross-check.
//
// Every public item here is `pub(crate)` unless its type shows up in a
// `pub` trait signature (`BddNode`) — the on-disk layout is an
// implementation detail of the `solver` crate, not something downstream
// consumers (interpreter, runtime) should depend on. The `Ccm` handle
// in `ccm.rs` is the stable public surface.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Domain-separation tag used by ADR-0005 §5 Step 5 to namespace the
/// `ccm_hash` pre-image so a byte string constructed for one ConfigFlux
/// hash chain cannot be substituted for another. The trailing `\n` is
/// part of the tag, not a separator — it locks the first byte of
/// whatever follows to the NUL-free ASCII range.
///
/// ADR-0005 Amendment 1 §14 / ADR-0012 §4: v2 ratchet path. The v1
/// pre-image cannot collide with a v2 pre-image because the domain tag
/// differs in the first 18 bytes of the SHA-256 input. Under the
/// v0.3.0 no-backcompat policy the solver only recognizes the v2 tag.
pub(crate) const CCM_HASH_DOMAIN_TAG: &[u8] = b"configflux.ccm.v2\n";

/// Maximum manifest schema version this build accepts. ADR-0005 §7
/// "solver built against schema versions (manifest_max: M, symbols_max: S)
/// accepts a CCM iff manifest.schema_version <= M AND
/// symbols.schema_version <= S". Bumping this is a deliberate event —
/// do not relax it without a corresponding schema_version bump in the
/// compiler's emitter.
///
/// ADR-0005 Amendment 1 §15: v2 ratchet. Solver-side accept range under
/// v0.3.0 is **strictly v2** (the no-backcompat policy drops v1
/// acceptance entirely; a v1 CCM cannot load against a v2 solver).
pub(crate) const CCM_MANIFEST_SCHEMA_MAX: u32 = 2;

/// Minimum manifest schema version this build accepts. ADR-0005
/// Amendment 1 §15 v0.3.0 no-backcompat: only v2 loads, v1 is dropped.
pub(crate) const CCM_MANIFEST_SCHEMA_MIN: u32 = 2;

/// Maximum symbols schema version this build accepts. Independent u32
/// from the manifest version per ADR-0005 §7. Bumping is a deliberate
/// event.
///
/// ADR-0005 Amendment 1 §15: v2 ratchet. Same no-backcompat reasoning
/// as `CCM_MANIFEST_SCHEMA_MAX`.
pub(crate) const CCM_SYMBOLS_SCHEMA_MAX: u32 = 2;

/// Minimum symbols schema version this build accepts.
pub(crate) const CCM_SYMBOLS_SCHEMA_MIN: u32 = 2;

/// ASCII magic `"CCMB"` at offset 0x00 of `ccm.bdd.bin` per ADR-0005 §4
/// file structure. Readers reject any file whose first four bytes are
/// not this sequence with `ParseError::BddBinParse`.
pub(crate) const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";

/// Format-version byte at offset 0x04 of `ccm.bdd.bin`. The same
/// rejection rule as the manifest schema version applies (§7 loud
/// rejection on over-range values).
pub(crate) const CCM_BDD_BIN_VERSION: u8 = 0x01;

/// Sentinel `var_index` marking a terminal node per ADR-0005 §4.
/// Terminal nodes carry this value to distinguish them from variable
/// nodes whose `var_index` is in `0..var_count`.
pub(crate) const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;

/// Sentinel used in `root_table` and in terminal `low_id`/`high_id`
/// back-pointers to denote "terminal FALSE" per ADR-0005 §4.
pub(crate) const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;

/// Sentinel used in `root_table` and in terminal `low_id`/`high_id`
/// back-pointers to denote "terminal TRUE" per ADR-0005 §4.
pub(crate) const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

/// Byte width of one `Node` record in `ccm.bdd.bin` per ADR-0005 §4
/// ("Total record width is exactly 16 bytes"). Any reader that walks
/// the node table strides by this constant; changing it is a schema
/// bump, not a refactor.
pub(crate) const NODE_RECORD_BYTES: usize = 16;

/// Width of the fixed-size header preceding the root table.
/// `[magic 4] + [version 1] + [reserved 3] + [var_count 4] +
/// [node_count 4] + [root_count 4]` = 20 bytes.
pub(crate) const BDD_BIN_HEADER_BYTES: usize = 20;

/// Recognized algorithm tags per ADR-0005 §2. Unknown tags are a loud
/// rejection per the same section. This is a compile-time whitelist —
/// adding a tag here is a deliberate review event that ships in the
/// same PR as the corresponding backend implementation.
pub(crate) const RECOGNIZED_ALGORITHMS: &[&str] = &[
    // ADR-0011 Amendment 1 §A1.1 (configflux-ew24): in-crate path,
    // replaces historical "robdd-oxidd-v1" (§A1.5 rotation; ADR-0001
    // §7 v0.3.0 no-stability-commitment).
    "robdd-handrolled-v1",
    // ADR-0011 + Amendment 1 §A1.1 (configflux-wbzw): CUDD-side
    // compiler build path. The actual emission lands with the new
    // single-importer module under compiler/src/cudd_build/.
    "robdd-cudd-v1",
    // "robdd-buddy-v1" — reserved per ADR-0005 §2 (BuDDy FFI fallback).
];

/// Error variants the format reader can raise. Maps to the stable
/// `CcmError` variants the public `Ccm` API exposes via a `From` impl
/// (see `ccm.rs`). Variants are kept narrow: each one is the exact
/// failure mode ADR-0005 names, so a caller that pattern-matches on
/// `CcmError` can distinguish a hash mismatch (§5 Load 6) from a
/// schema-version rejection (§7).
///
/// The attached message strings on `Io` and `ManifestParse` are not
/// threaded through to the public `CcmError` enum (it is a narrow
/// stable-variant enum); they exist so `#[derive(Debug)]` can render a
/// developer-friendly error when a test panics on an unexpected parse
/// failure. `#[allow(dead_code)]` is load-bearing: rustc's dead-code
/// lint does not count `Debug`-only use of a field as a real read, and
/// the `-D warnings` configuration in some build modes would reject
/// the file without this allowance.
#[allow(dead_code)]
#[derive(Debug)]
pub(crate) enum ParseError {
    /// An expected file on disk is missing or unreadable. Wraps a
    /// `std::io::Error` message in a `String` to keep `ParseError`
    /// `Clone`-free without introducing an `io::Error` dependency in
    /// the public `CcmError` enum.
    Io(String),
    /// `ccm.manifest.json` or `ccm.symbols.json` failed to parse as
    /// JSON, or a required field was absent.
    ManifestParse(String),
    /// `ccm.bdd.bin` failed a structural check (bad magic, truncated
    /// record, reserved bit set, non-reduced).
    BddBinParse(&'static str),
    /// The manifest or symbols `schema_version` is higher than this
    /// build supports. ADR-0005 §7 requires a loud rejection.
    UnsupportedSchemaVersion,
    /// The `algorithm` tag is not in the compile-time whitelist.
    /// ADR-0005 §2 requires a loud rejection.
    UnknownAlgorithm,
    /// The computed `ccm_hash` does not match the one stored in the
    /// manifest. ADR-0005 §5 Load 6.
    HashMismatch,
}

/// On-disk layout of `ccm.manifest.json` per ADR-0005 §2.
///
/// `schema_version` is read first and rejected before the rest of the
/// struct is interpreted; this avoids reading fields whose semantics
/// may have shifted in a future schema. `bound_model_hash` is the
/// load-time integrity check per §9 (solver-facing; the cross-CMP
/// comparison is done by the caller, not by this struct).
///
/// The `Default`-free field set is deliberate: ADR-0005 §2 requires
/// every field to be present in every emission. A missing field is a
/// `ManifestParse` error, not a silent default.
///
/// ADR-0005 Amendment 1 §12: under v2 the **top-level**
/// `ccm.manifest.json` gains a required `partition_manifest` field. The
/// per-partition manifests do NOT carry this field. We model that with
/// a single struct whose `partition_manifest` is `Option<String>`:
/// `None` after parsing means "per-partition manifest" (no field
/// present); `Some(_)` means "top-level multi-part manifest". The
/// loader cross-checks the field's presence against where it found the
/// manifest in the directory tree (`<out>/ccm/` for top-level vs
/// `<out>/ccm/partition-NNNN/` for per-partition).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub schema_version: u32,
    pub ccm_hash: String,
    pub bound_model_hash: String,
    pub algorithm: String,
    pub algorithm_params: BTreeMap<String, String>,
    pub var_count: u32,
    pub node_count: u64,
    pub construction_wall_time_us: u64,
    pub emitted_at: String,
    /// v2-only top-level field per ADR-0005 Amendment 1 §12. Present on
    /// the multi-part top-level manifest; absent on per-partition
    /// manifests. `serde(default)` keeps the deserializer
    /// backwards-compatible at the type level (the loader enforces
    /// presence/absence per directory).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_manifest: Option<String>,
    /// ADR-0054 §5.4 constraint roster: the authored policy conjuncts of the
    /// BDD root, with the `root_index` that maps a minimal-unsat-core position
    /// back to a constraint id. Top-level only — per-partition manifests never
    /// carry it, and synthesized intra-facet cardinality is never in it.
    ///
    /// The roster is INSIDE the top-level hash pre-image, so it must be
    /// round-tripped exactly: `serde(default)` for a model that declares no
    /// constraint (the emitter omits the field entirely rather than writing an
    /// empty array, which keeps every pre-ADR-0054 artifact's `ccm_hash`
    /// unmoved), and `skip_serializing_if` so the reconstructed pre-image
    /// reproduces the compiler's canonical bytes in both cases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub constraints: Vec<ConstraintRosterEntry>,
}

/// One entry of the ADR-0054 §5.4 constraint roster, as written by
/// `compiler/src/ccm_emitter/multi_part.rs`. Field order is lexicographic
/// because these bytes re-enter the top-level `ccm_hash` pre-image.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConstraintRosterEntry {
    /// The authored condition text, verbatim.
    pub condition: String,
    /// The authored `constraint_id`.
    pub id: String,
    /// Position in the root AND-fold among the authored conjuncts.
    pub root_index: u32,
}

/// On-disk layout of `ccm.symbols.json` per ADR-0005 §3.
///
/// The parse path checks the `facet_to_var[variable_order[i]] == i`
/// consistency invariant (§3 "mismatch is a stable error") before
/// handing the struct back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Symbols {
    pub schema_version: u32,
    pub variable_order: Vec<String>,
    pub facet_to_var: BTreeMap<String, u32>,
    pub var_to_label: Vec<String>,
}

/// A single BDD node decoded from the node table in `ccm.bdd.bin`.
///
/// Fields mirror the ADR-0005 §4 on-disk layout exactly. Terminal
/// nodes carry `var_index == TERMINAL_VAR_INDEX` and use `low_id`/
/// `high_id` as self-referential back-pointers (the reader ignores
/// those back-pointers for variable-lookup purposes; they exist only
/// to keep the record width fixed).
///
/// `BddNode` is `pub` because it appears in the signature of the
/// `SolverBackend::deserialize_bdd` trait method (which is also `pub`
/// so external-crate backends can implement it in the future).
/// Downstream consumers should not construct `BddNode` values by hand;
/// they come from `Ccm::bdd()` after a successful `load_ccm`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BddNode {
    pub var_index: u32,
    pub low_id: u32,
    pub high_id: u32,
    pub flags: u8,
}

/// The decoded `ccm.bdd.bin` payload. Owns the full node table plus
/// the root indices; no lazy streaming is done because at the 100k
/// target the whole file fits in the tens-of-MB range (ADR-0005 §4
/// "No compression... acceptable for a compiled artifact").
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BddPayload {
    pub var_count: u32,
    pub node_count: u32,
    pub roots: Vec<u32>,
    pub nodes: Vec<BddNode>,
}

/// Fully-parsed CCM artifact triple. The `Ccm` handle in `ccm.rs`
/// stores a projection of this (hash + schema + symbols + BDD);
/// `ccm_format` is the only consumer that sees the full tuple.
///
/// The `manifest` field is intentionally kept even though current
/// readers consume only the cross-checked hash bytes — it lets a
/// future diagnostic or `explain_rejection` caller render the
/// manifest contents without re-parsing.
#[derive(Debug, Clone)]
pub(crate) struct ParsedCcm {
    #[allow(dead_code)]
    pub manifest: Manifest,
    pub symbols: Symbols,
    pub bdd: BddPayload,
    pub ccm_hash_bytes: [u8; 32],
    pub bound_model_hash_bytes: [u8; 32],
}

/// Resolve a caller-provided path to the three sibling files defined
/// by ADR-0005 §1. The input may be either:
///
///   - a path to a `ccm.manifest.json` file (in which case the two
///     siblings live in the same directory), or
///   - a path to a `ccm/` directory (in which case all three files
///     live inside it).
///
/// Returns `None` if neither interpretation resolves to a readable
/// manifest file; the caller treats `None` as "no CCM to load" and
/// falls back to the empty-Ccm stub (this path is how
/// `round_trip_empty.rs`'s `UNUSED_CCM_PATH` sentinel keeps working
/// post-8dm.2).
pub(crate) fn resolve_ccm_dir(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        // Treat as a manifest file path; use its parent directory.
        path.parent()?.to_path_buf()
    };
    // A CCM directory is one that contains all three ADR-0005 §1 files.
    // Missing any of them is an "incomplete CCM" state we treat as a
    // parse error at load time (see `load_ccm_from_dir`). This helper
    // only confirms the directory existence and returns the path; the
    // full three-file check happens in the loader below.
    if dir.join("ccm.manifest.json").is_file() {
        Some(dir)
    } else {
        None
    }
}

/// Parse the full three-file CCM from a directory. The directory must
/// contain `ccm.manifest.json`, `ccm.symbols.json`, and `ccm.bdd.bin`
/// per ADR-0005 §1.
///
/// Steps:
///   1. Read manifest JSON bytes; parse `schema_version` only.
///   2. Reject if `schema_version > CCM_MANIFEST_SCHEMA_MAX` (§7).
///   3. Parse the rest of the manifest.
///   4. Reject if `algorithm` not in `RECOGNIZED_ALGORITHMS` (§2).
///   5. Read symbols JSON bytes; parse and verify internal
///      consistency (§3).
///   6. Read BDD binary bytes; parse and verify reducedness (§4).
///   7. Re-compute `ccm_hash` per §5 Step 5-6 and compare against
///      `manifest.ccm_hash`; reject on mismatch.
///   8. Decode `bound_model_hash` from hex.
pub(crate) fn load_ccm_from_dir(dir: &Path) -> Result<ParsedCcm, ParseError> {
    let manifest_path = dir.join("ccm.manifest.json");
    let symbols_path = dir.join("ccm.symbols.json");
    let bdd_path = dir.join("ccm.bdd.bin");

    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|e| ParseError::Io(format!("read {}: {e}", manifest_path.display())))?;
    let symbols_bytes = fs::read(&symbols_path)
        .map_err(|e| ParseError::Io(format!("read {}: {e}", symbols_path.display())))?;
    let bdd_bytes = fs::read(&bdd_path)
        .map_err(|e| ParseError::Io(format!("read {}: {e}", bdd_path.display())))?;

    // Step 1-4: parse and validate manifest.
    let manifest = parse_manifest(&manifest_bytes)?;
    // ADR-0005 Amendment 1 §15: v0.3.0 no-backcompat — strictly v2.
    if manifest.schema_version < CCM_MANIFEST_SCHEMA_MIN
        || manifest.schema_version > CCM_MANIFEST_SCHEMA_MAX
    {
        return Err(ParseError::UnsupportedSchemaVersion);
    }
    if !RECOGNIZED_ALGORITHMS.contains(&manifest.algorithm.as_str()) {
        return Err(ParseError::UnknownAlgorithm);
    }
    // ADR-0005 Amendment 1 §12: per-partition manifests do NOT carry the
    // `partition_manifest` field. Its presence here is a hard error
    // because mwyp's per-partition load path is called with the
    // partition subdirectory, never the top-level directory.
    if manifest.partition_manifest.is_some() {
        return Err(ParseError::ManifestParse(
            "per-partition ccm.manifest.json must not carry the partition_manifest field".into(),
        ));
    }
    // ADR-0054 §5.4: the constraint roster is model-global and lives only on
    // the top-level manifest, which is what keeps the partitioning scheme out
    // of constraint identity. A roster here would mean a `root_index` scoped to
    // a partition, which no consumer could interpret — reject it rather than
    // silently ignore it, symmetrically with `partition_manifest` above.
    if !manifest.constraints.is_empty() {
        return Err(ParseError::ManifestParse(
            "per-partition ccm.manifest.json must not carry the constraints roster".into(),
        ));
    }

    // Step 5: parse and validate symbols.
    let symbols = parse_symbols(&symbols_bytes)?;
    if symbols.schema_version < CCM_SYMBOLS_SCHEMA_MIN
        || symbols.schema_version > CCM_SYMBOLS_SCHEMA_MAX
    {
        return Err(ParseError::UnsupportedSchemaVersion);
    }
    verify_symbols_consistency(&symbols, manifest.var_count)?;

    // Step 6: parse and validate BDD binary.
    let bdd = parse_bdd_bin(&bdd_bytes, manifest.var_count, manifest.node_count)?;

    // Step 7: recompute ccm_hash per §5 and compare.
    let computed = compute_ccm_hash(&manifest, &symbols_bytes, &bdd_bytes)?;
    let claimed = decode_hex32(&manifest.ccm_hash).ok_or_else(|| {
        ParseError::ManifestParse("ccm_hash: not 64-char lowercase hex".into())
    })?;
    if computed != claimed {
        return Err(ParseError::HashMismatch);
    }

    let bound_model_hash_bytes = decode_hex32(&manifest.bound_model_hash).ok_or_else(|| {
        ParseError::ManifestParse("bound_model_hash: not 64-char lowercase hex".into())
    })?;

    Ok(ParsedCcm {
        manifest,
        symbols,
        bdd,
        ccm_hash_bytes: computed,
        bound_model_hash_bytes,
    })
}

/// Parse a `ccm.manifest.json` byte buffer into `Manifest`.
///
/// On success every field named in ADR-0005 §2 is populated; a missing
/// field is a `ManifestParse` error, not a silent default.
pub(crate) fn parse_manifest(bytes: &[u8]) -> Result<Manifest, ParseError> {
    serde_json::from_slice::<Manifest>(bytes)
        .map_err(|e| ParseError::ManifestParse(format!("ccm.manifest.json: {e}")))
}

/// Parse a `ccm.symbols.json` byte buffer into `Symbols`.
pub(crate) fn parse_symbols(bytes: &[u8]) -> Result<Symbols, ParseError> {
    serde_json::from_slice::<Symbols>(bytes)
        .map_err(|e| ParseError::ManifestParse(format!("ccm.symbols.json: {e}")))
}

/// Cross-check that `symbols.facet_to_var` is the inverse of
/// `symbols.variable_order` and that both have length equal to the
/// manifest's advertised `var_count`.
fn verify_symbols_consistency(symbols: &Symbols, manifest_var_count: u32) -> Result<(), ParseError> {
    if symbols.variable_order.len() != manifest_var_count as usize {
        return Err(ParseError::ManifestParse(
            "ccm.symbols.json: variable_order length does not match manifest.var_count".into(),
        ));
    }
    if symbols.var_to_label.len() != manifest_var_count as usize {
        return Err(ParseError::ManifestParse(
            "ccm.symbols.json: var_to_label length does not match manifest.var_count".into(),
        ));
    }
    if symbols.facet_to_var.len() != manifest_var_count as usize {
        return Err(ParseError::ManifestParse(
            "ccm.symbols.json: facet_to_var length does not match manifest.var_count".into(),
        ));
    }
    for (i, facet) in symbols.variable_order.iter().enumerate() {
        match symbols.facet_to_var.get(facet) {
            Some(&idx) if idx as usize == i => {}
            Some(_) => {
                return Err(ParseError::ManifestParse(format!(
                    "ccm.symbols.json: facet_to_var[{facet}] does not equal its variable_order index"
                )));
            }
            None => {
                return Err(ParseError::ManifestParse(format!(
                    "ccm.symbols.json: facet_to_var missing entry for {facet}"
                )));
            }
        }
    }
    Ok(())
}

/// Parse the `ccm.bdd.bin` byte buffer. Verifies the magic, version,
/// reserved bytes, header width, record width, and the reducedness
/// invariant (§4 "no node has low_id == high_id; no two distinct
/// non-terminals share the same (var_index, low_id, high_id, flags)").
pub(crate) fn parse_bdd_bin(
    bytes: &[u8],
    manifest_var_count: u32,
    manifest_node_count: u64,
) -> Result<BddPayload, ParseError> {
    if bytes.len() < BDD_BIN_HEADER_BYTES {
        return Err(ParseError::BddBinParse("header truncated"));
    }
    if &bytes[0..4] != CCM_BDD_BIN_MAGIC {
        return Err(ParseError::BddBinParse("bad magic: expected CCMB"));
    }
    let version = bytes[4];
    if version != CCM_BDD_BIN_VERSION {
        return Err(ParseError::BddBinParse("unsupported ccm.bdd.bin version"));
    }
    // Reserved bytes at offsets 0x05..0x08 MUST be zero per §4.
    if bytes[5] != 0 || bytes[6] != 0 || bytes[7] != 0 {
        return Err(ParseError::BddBinParse("reserved header byte is nonzero"));
    }
    let var_count = read_u32_le(&bytes[8..12]);
    let node_count = read_u32_le(&bytes[12..16]);
    let root_count = read_u32_le(&bytes[16..20]);

    if var_count != manifest_var_count {
        return Err(ParseError::BddBinParse(
            "ccm.bdd.bin var_count does not match manifest.var_count",
        ));
    }
    if node_count as u64 != manifest_node_count {
        return Err(ParseError::BddBinParse(
            "ccm.bdd.bin node_count does not match manifest.node_count",
        ));
    }
    if root_count < 1 {
        return Err(ParseError::BddBinParse(
            "ccm.bdd.bin root_count must be >= 1",
        ));
    }

    let roots_start = BDD_BIN_HEADER_BYTES;
    let roots_end = roots_start
        .checked_add(
            4usize
                .checked_mul(root_count as usize)
                .ok_or(ParseError::BddBinParse("root_count * 4 overflow"))?,
        )
        .ok_or(ParseError::BddBinParse("root table offset overflow"))?;
    if bytes.len() < roots_end {
        return Err(ParseError::BddBinParse("root table truncated"));
    }
    let mut roots = Vec::with_capacity(root_count as usize);
    for i in 0..root_count as usize {
        let off = roots_start + i * 4;
        roots.push(read_u32_le(&bytes[off..off + 4]));
    }

    let nodes_start = roots_end;
    let nodes_end = nodes_start
        .checked_add(
            NODE_RECORD_BYTES
                .checked_mul(node_count as usize)
                .ok_or(ParseError::BddBinParse("node_count * 16 overflow"))?,
        )
        .ok_or(ParseError::BddBinParse("node table offset overflow"))?;
    if bytes.len() != nodes_end {
        return Err(ParseError::BddBinParse(
            "ccm.bdd.bin has trailing or missing bytes after node table",
        ));
    }

    let mut nodes = Vec::with_capacity(node_count as usize);
    for i in 0..node_count as usize {
        let off = nodes_start + i * NODE_RECORD_BYTES;
        let var_index = read_u32_le(&bytes[off..off + 4]);
        let low_id = read_u32_le(&bytes[off + 4..off + 8]);
        let high_id = read_u32_le(&bytes[off + 8..off + 12]);
        let flags = bytes[off + 12];
        // ADR-0005 §4: "bits 1-7 reserved, MUST be 0... file with a
        // non-zero reserved bit is rejected"
        if flags & 0b1111_1110 != 0 {
            return Err(ParseError::BddBinParse("node flags: reserved bit set"));
        }
        // ADR-0005 §4: the three padding bytes MUST be 0x00.
        if bytes[off + 13] != 0 || bytes[off + 14] != 0 || bytes[off + 15] != 0 {
            return Err(ParseError::BddBinParse(
                "node pad: nonzero byte in reserved padding",
            ));
        }
        nodes.push(BddNode {
            var_index,
            low_id,
            high_id,
            flags,
        });
    }

    // Structural checks:
    //   - var_index on non-terminals must be in [0, var_count).
    //   - low/high indices on non-terminals must point at a strictly
    //     smaller index (post-order / children-before-parents per §4).
    //   - reducedness: low_id != high_id on any non-terminal; no two
    //     non-terminals share (var_index, low_id, high_id, flags).
    for (i, node) in nodes.iter().enumerate() {
        if node.var_index == TERMINAL_VAR_INDEX {
            // Terminal. §4 fixes that index 0 is FALSE and index 1 is TRUE.
            if i == 0 {
                if node.low_id != TERMINAL_FALSE || node.high_id != TERMINAL_FALSE {
                    return Err(ParseError::BddBinParse(
                        "terminal FALSE backpointers not 0xFFFFFFFF",
                    ));
                }
            } else if i == 1 {
                if node.low_id != TERMINAL_TRUE || node.high_id != TERMINAL_TRUE {
                    return Err(ParseError::BddBinParse(
                        "terminal TRUE backpointers not 0xFFFFFFFE",
                    ));
                }
            } else {
                return Err(ParseError::BddBinParse(
                    "terminal-tagged node at unexpected index",
                ));
            }
            continue;
        }
        if node.var_index >= var_count {
            return Err(ParseError::BddBinParse("node var_index out of range"));
        }
        if node.low_id == node.high_id {
            return Err(ParseError::BddBinParse("non-reduced: low_id == high_id"));
        }
        // Cross-check child indices: sentinels are OK, otherwise must
        // point strictly earlier in the table (post-order).
        for &child in [node.low_id, node.high_id].iter() {
            if child == TERMINAL_FALSE || child == TERMINAL_TRUE {
                continue;
            }
            if child as usize >= i {
                return Err(ParseError::BddBinParse(
                    "non-post-order: child index >= parent index",
                ));
            }
        }
    }

    // Reducedness invariant #2: no two distinct non-terminals share the
    // same (var_index, low_id, high_id, flags) tuple. Quadratic in the
    // worst case; for the scale target (100k vars, millions of nodes) a
    // later task replaces this with a hashmap lookup, but at 8dm.2 the
    // fixtures are tiny and clarity beats cleverness.
    for (i, a) in nodes.iter().enumerate() {
        if a.var_index == TERMINAL_VAR_INDEX {
            continue;
        }
        for b in nodes.iter().take(i) {
            if b.var_index == TERMINAL_VAR_INDEX {
                continue;
            }
            if a.var_index == b.var_index
                && a.low_id == b.low_id
                && a.high_id == b.high_id
                && a.flags == b.flags
            {
                return Err(ParseError::BddBinParse(
                    "non-reduced: duplicate (var_index, low_id, high_id, flags)",
                ));
            }
        }
    }

    Ok(BddPayload {
        var_count,
        node_count,
        roots,
        nodes,
    })
}

/// Compute a **per-partition** `ccm_hash` pre-image per ADR-0005 §5
/// Step 5-6, re-tagged for v2 per ADR-0005 Amendment 1 §14 / ADR-0012 §5.
///
/// The pre-image is:
///
/// ```text
///   "configflux.ccm.v2\n"               (v2 domain separation tag)
///     || canonical(manifest with ccm_hash, construction_wall_time_us, emitted_at elided)
///     || "\n"
///     || SHA-256(ccm.bdd.bin bytes)     (32 raw bytes)
///     || SHA-256(ccm.symbols.json bytes) (32 raw bytes)
/// ```
///
/// The per-partition manifest does NOT carry the v2-only
/// `partition_manifest` field (ADR-0005 Amendment 1 §12), so the
/// pre-image's hand-rolled `PreImageManifest` struct does not list it.
/// The top-level pre-image is computed separately by
/// [`compute_top_level_ccm_hash`] below because it MUST include
/// `partition_manifest` in its canonical bytes (ADR-0012 §5 Step 3).
///
/// We re-use the caller's raw file bytes for the bdd/symbols digests
/// rather than re-serializing; the §5 procedure explicitly hashes the
/// as-written file bytes (Step 1-2).
fn compute_ccm_hash(
    manifest: &Manifest,
    symbols_bytes: &[u8],
    bdd_bytes: &[u8],
) -> Result<[u8; 32], ParseError> {
    // Build the pre-image manifest per §5 Step 3: elide ccm_hash,
    // construction_wall_time_us, emitted_at. We use a separate struct
    // rather than mutating `Manifest` in place so the caller keeps its
    // full on-disk view.
    //
    // `BTreeMap` preserves key order under serde_json's default
    // serializer, matching the §6 rule 2 lexicographic ordering
    // requirement. The field order inside this struct is lexicographic
    // as well — serde_json serializes in declaration order, so we
    // declare in the same order that canonical output requires.
    #[derive(Serialize)]
    struct PreImageManifest<'a> {
        algorithm: &'a str,
        algorithm_params: &'a BTreeMap<String, String>,
        bound_model_hash: &'a str,
        node_count: u64,
        schema_version: u32,
        var_count: u32,
    }
    let pre = PreImageManifest {
        algorithm: &manifest.algorithm,
        algorithm_params: &manifest.algorithm_params,
        bound_model_hash: &manifest.bound_model_hash,
        node_count: manifest.node_count,
        schema_version: manifest.schema_version,
        var_count: manifest.var_count,
    };
    let manifest_canon = serde_json::to_vec(&pre)
        .map_err(|e| ParseError::ManifestParse(format!("preimage serialize failed: {e}")))?;

    let bdd_digest = sha256(bdd_bytes);
    let symbols_digest = sha256(symbols_bytes);

    let mut hasher = Sha256::new();
    hasher.update(CCM_HASH_DOMAIN_TAG);
    hasher.update(&manifest_canon);
    hasher.update(b"\n");
    hasher.update(&bdd_digest);
    hasher.update(&symbols_digest);
    let out: [u8; 32] = hasher.finalize().into();
    Ok(out)
}

/// Compute the SHA-256 digest of `bytes` and return the raw 32-byte
/// array. Extracted to a helper so the ccm_hash and per-file hash code
/// paths share a single implementation.
pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

/// Encode a 32-byte SHA-256 digest as a 64-character lowercase hex
/// string per ADR-0005 §2 "every hash field ... is a lowercase
/// hex-encoded SHA-256 digest".
///
/// Hand-rolled (~10 lines) rather than pulling in the `hex` crate to
/// avoid forcing a crate_universe re-pin for a trivial function. The
/// output is byte-for-byte the same as `hex::encode`'s lowercase
/// emission; the round-trip test at the bottom of this file pins the
/// behavior.
///
/// Currently unused in non-test code — kept `pub(crate)` so the
/// downstream `valid_options` task (configflux-8dm.3) can reuse it
/// when formatting BDD variable labels in `explain_rejection` output.
#[allow(dead_code)]
pub(crate) fn encode_hex32(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for &b in bytes.iter() {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// Decode a 64-character lowercase hex string into a 32-byte digest.
/// Returns `None` on any deviation (wrong length, non-hex character,
/// uppercase letter — §6 rule 5 pins lowercase for string escapes and
/// §2 pins lowercase for hash fields).
///
/// Rejecting uppercase here is load-bearing: a future compiler emitter
/// must write the digest as lowercase per ADR-0005 §2, and a CCM whose
/// hex digits are uppercase would silently become a different byte
/// string under `ccm_hash` pre-image recomputation. The loader's job
/// is to refuse them so a drifted emitter is caught immediately, not
/// to quietly paper over the case mismatch.
pub(crate) fn decode_hex32(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    let bytes = s.as_bytes();
    for i in 0..32 {
        let hi = hex_digit_lower(bytes[i * 2])?;
        let lo = hex_digit_lower(bytes[i * 2 + 1])?;
        out[i] = (hi << 4) | lo;
    }
    Some(out)
}

/// Decode one lowercase hex digit. Returns `None` for uppercase A-F,
/// non-hex ASCII, or non-ASCII bytes. See `decode_hex32` above for the
/// rationale on the lowercase-only rule.
fn hex_digit_lower(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        _ => None,
    }
}

#[inline]
fn read_u32_le(b: &[u8]) -> u32 {
    debug_assert!(b.len() >= 4);
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Compute the top-level `ccm_hash` per ADR-0012 §5 Step 3-4.
///
/// The pre-image extends the per-partition recipe to cover the
/// multi-part artifact as a whole:
///
/// ```text
///   "configflux.ccm.v2\n"                (v2 domain tag)
///     || top_level_manifest_canon        (canonical bytes of the
///                                         top-level manifest WITH
///                                         `partition_manifest` field
///                                         INCLUDED but with `ccm_hash`,
///                                         `construction_wall_time_us`,
///                                         and `emitted_at` elided)
///     || "\n"
///     || partition_manifest_canon        (canonical bytes of
///                                         partition-manifest.json with
///                                         `top_level_ccm_hash` elided)
///     || "\n"
///     || partition_0.ccm_hash_raw        (32 raw bytes)
///     || partition_1.ccm_hash_raw
///     || ...
///     || partition_N-1.ccm_hash_raw
///     || bridge.ccm_hash_raw             (32 raw bytes, present iff has_bridge)
/// ```
///
/// The caller must:
///   - supply the top-level [`Manifest`] (with `partition_manifest = Some(...)`),
///   - supply the partition-manifest fields (schema_version, partitions,
///     has_bridge) so the canonical pre-image matches what the compiler
///     emitted, byte-for-byte,
///   - supply the per-partition raw 32-byte ccm_hashes in the same order
///     the partition manifest lists them.
pub(crate) fn compute_top_level_ccm_hash(
    manifest: &Manifest,
    pm_schema_version: u32,
    partitions: &[String],
    has_bridge: bool,
    per_partition_hashes: &[[u8; 32]],
) -> Result<[u8; 32], ParseError> {
    let Some(partition_manifest_path) = manifest.partition_manifest.as_deref() else {
        return Err(ParseError::ManifestParse(
            "top-level manifest missing required partition_manifest field".into(),
        ));
    };

    // ADR-0012 §5 Step 3: top-level pre-image includes
    // `partition_manifest` but elides `ccm_hash`, `construction_wall_time_us`,
    // `emitted_at`. Lexicographic field order matches the compiler's
    // emission shape (see compiler/src/ccm_emitter/multi_part.rs
    // `TopLevelPreimage`).
    // ADR-0054 §5.4: the constraint roster is part of the top-level pre-image
    // (a model's policy is part of its content address), and is omitted
    // entirely when empty. Both halves matter here: omitting it keeps every
    // constraint-free artifact's ccm_hash exactly where it was, and including
    // it when present is what makes a roster tamper detectable on load.
    #[derive(Serialize)]
    struct TopLevelPreImage<'a> {
        algorithm: &'a str,
        algorithm_params: &'a BTreeMap<String, String>,
        bound_model_hash: &'a str,
        #[serde(skip_serializing_if = "<[ConstraintRosterEntry]>::is_empty")]
        constraints: &'a [ConstraintRosterEntry],
        node_count: u64,
        partition_manifest: &'a str,
        schema_version: u32,
        var_count: u32,
    }
    let top_pre = TopLevelPreImage {
        algorithm: &manifest.algorithm,
        algorithm_params: &manifest.algorithm_params,
        bound_model_hash: &manifest.bound_model_hash,
        constraints: &manifest.constraints,
        node_count: manifest.node_count,
        partition_manifest: partition_manifest_path,
        schema_version: manifest.schema_version,
        var_count: manifest.var_count,
    };
    let top_canon = serde_json::to_vec(&top_pre)
        .map_err(|e| ParseError::ManifestParse(format!("top-level preimage serialize: {e}")))?;

    // ADR-0012 §5 Step 2: partition-manifest pre-image with
    // `top_level_ccm_hash` elided. The compiler's emitter
    // (multi_part.rs `PartitionManifestPreimage`) uses lexicographic
    // field order; mirror it.
    #[derive(Serialize)]
    struct PartitionManifestPreImage<'a> {
        has_bridge: bool,
        partitions: &'a [String],
        schema_version: u32,
    }
    let pm_pre = PartitionManifestPreImage {
        has_bridge,
        partitions,
        schema_version: pm_schema_version,
    };
    let pm_canon = serde_json::to_vec(&pm_pre)
        .map_err(|e| ParseError::ManifestParse(format!("partition-manifest preimage serialize: {e}")))?;

    let mut hasher = Sha256::new();
    hasher.update(CCM_HASH_DOMAIN_TAG);
    hasher.update(&top_canon);
    hasher.update(b"\n");
    hasher.update(&pm_canon);
    hasher.update(b"\n");
    for h in per_partition_hashes {
        hasher.update(h);
    }
    let raw: [u8; 32] = hasher.finalize().into();
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_accepts_lowercase_only() {
        let bytes = [0x0au8; 32];
        let encoded = encode_hex32(&bytes);
        assert_eq!(encoded.len(), 64);
        assert!(encoded.bytes().all(|b| !b.is_ascii_uppercase()));
        let decoded = decode_hex32(&encoded).expect("round trip");
        assert_eq!(decoded, bytes);
    }

    #[test]
    fn hex_encodes_known_byte_pattern() {
        // Pin the encoder against a known string so a refactor cannot
        // silently change the byte layout. [0x00..=0x1f] fits in the
        // first 32 bytes and covers every nibble transition.
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = i as u8;
        }
        let encoded = encode_hex32(&bytes);
        assert_eq!(
            encoded,
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        );
    }

    #[test]
    fn hex_decode_rejects_uppercase() {
        // Uppercase must be rejected. This enforces ADR-0005 §2's
        // lowercase pin.
        let s = "AA".to_string() + &"00".repeat(31);
        assert_eq!(decode_hex32(&s), None);
    }

    #[test]
    fn hex_decode_rejects_wrong_length() {
        assert_eq!(decode_hex32(""), None);
        assert_eq!(decode_hex32(&"00".repeat(31)), None);
        assert_eq!(decode_hex32(&"00".repeat(33)), None);
    }

    #[test]
    fn hex_decode_rejects_nonhex_char() {
        let s = "gg".to_string() + &"00".repeat(31);
        assert_eq!(decode_hex32(&s), None);
    }

    #[test]
    fn resolve_ccm_dir_returns_none_for_nonexistent_path() {
        assert!(resolve_ccm_dir(Path::new("/nope/does/not/exist.ccm")).is_none());
    }

    #[test]
    fn parse_bdd_bin_rejects_wrong_magic() {
        let mut bytes = vec![0u8; BDD_BIN_HEADER_BYTES];
        bytes[0..4].copy_from_slice(b"XXXX");
        bytes[4] = CCM_BDD_BIN_VERSION;
        let err = parse_bdd_bin(&bytes, 0, 0).unwrap_err();
        assert!(matches!(err, ParseError::BddBinParse(msg) if msg.contains("magic")));
    }

    #[test]
    fn parse_bdd_bin_rejects_wrong_version() {
        let mut bytes = vec![0u8; BDD_BIN_HEADER_BYTES];
        bytes[0..4].copy_from_slice(CCM_BDD_BIN_MAGIC);
        bytes[4] = 0xff;
        let err = parse_bdd_bin(&bytes, 0, 0).unwrap_err();
        assert!(matches!(err, ParseError::BddBinParse(msg) if msg.contains("version")));
    }

    #[test]
    fn parse_bdd_bin_rejects_nonzero_reserved_header_byte() {
        let mut bytes = vec![0u8; BDD_BIN_HEADER_BYTES];
        bytes[0..4].copy_from_slice(CCM_BDD_BIN_MAGIC);
        bytes[4] = CCM_BDD_BIN_VERSION;
        bytes[5] = 0x42;
        let err = parse_bdd_bin(&bytes, 0, 0).unwrap_err();
        assert!(matches!(err, ParseError::BddBinParse(msg) if msg.contains("reserved")));
    }

    /// Build a valid empty-CCM `ccm.bdd.bin`: two terminal records
    /// (FALSE at 0, TRUE at 1) with root_table = [TERMINAL_TRUE] so the
    /// formula is trivially "satisfiable with the empty assignment"
    /// (ADR-0005 §4 "root_table[0] is the appropriate terminal sentinel
    /// if the formula is trivially true or false").
    fn build_empty_bdd_bin() -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
        bytes.push(CCM_BDD_BIN_VERSION);
        bytes.extend_from_slice(&[0u8; 3]); // reserved
        bytes.extend_from_slice(&0u32.to_le_bytes()); // var_count
        bytes.extend_from_slice(&2u32.to_le_bytes()); // node_count
        bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count
        bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes()); // root[0]
        // Terminal FALSE at index 0
        bytes.extend_from_slice(&TERMINAL_VAR_INDEX.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_FALSE.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_FALSE.to_le_bytes());
        bytes.push(0u8); // flags
        bytes.extend_from_slice(&[0u8; 3]); // pad
        // Terminal TRUE at index 1
        bytes.extend_from_slice(&TERMINAL_VAR_INDEX.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
        bytes.push(0u8); // flags
        bytes.extend_from_slice(&[0u8; 3]); // pad
        bytes
    }

    #[test]
    fn parse_bdd_bin_accepts_empty_trivial_true_formula() {
        let bytes = build_empty_bdd_bin();
        let payload = parse_bdd_bin(&bytes, 0, 2).expect("empty-true bdd parses");
        assert_eq!(payload.var_count, 0);
        assert_eq!(payload.node_count, 2);
        assert_eq!(payload.roots, vec![TERMINAL_TRUE]);
        assert_eq!(payload.nodes.len(), 2);
        assert_eq!(payload.nodes[0].var_index, TERMINAL_VAR_INDEX);
        assert_eq!(payload.nodes[1].var_index, TERMINAL_VAR_INDEX);
    }

    #[test]
    fn parse_bdd_bin_rejects_trailing_bytes() {
        let mut bytes = build_empty_bdd_bin();
        bytes.push(0xab);
        let err = parse_bdd_bin(&bytes, 0, 2).unwrap_err();
        assert!(
            matches!(err, ParseError::BddBinParse(msg) if msg.contains("trailing") || msg.contains("missing"))
        );
    }

    #[test]
    fn parse_bdd_bin_rejects_reducedness_violation_low_equals_high() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
        bytes.push(CCM_BDD_BIN_VERSION);
        bytes.extend_from_slice(&[0u8; 3]);
        bytes.extend_from_slice(&1u32.to_le_bytes()); // var_count
        bytes.extend_from_slice(&3u32.to_le_bytes()); // node_count
        bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count
        bytes.extend_from_slice(&2u32.to_le_bytes()); // root[0] -> node 2
        // Terminal FALSE at 0
        bytes.extend_from_slice(&TERMINAL_VAR_INDEX.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_FALSE.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_FALSE.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&[0u8; 3]);
        // Terminal TRUE at 1
        bytes.extend_from_slice(&TERMINAL_VAR_INDEX.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&[0u8; 3]);
        // Non-terminal at 2 with low_id == high_id (reducedness violation)
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
        bytes.extend_from_slice(&TERMINAL_TRUE.to_le_bytes());
        bytes.push(0);
        bytes.extend_from_slice(&[0u8; 3]);

        let err = parse_bdd_bin(&bytes, 1, 3).unwrap_err();
        assert!(
            matches!(err, ParseError::BddBinParse(msg) if msg.contains("low_id == high_id")),
            "expected reducedness rejection, got {err:?}"
        );
    }

    #[test]
    fn parse_symbols_verifies_inverse_mapping() {
        let good = r#"{
            "schema_version": 1,
            "variable_order": ["a", "b"],
            "facet_to_var": {"a": 0, "b": 1},
            "var_to_label": ["A", "B"]
        }"#;
        let s = parse_symbols(good.as_bytes()).expect("good symbols parses");
        verify_symbols_consistency(&s, 2).expect("consistency holds");

        let bad = r#"{
            "schema_version": 1,
            "variable_order": ["a", "b"],
            "facet_to_var": {"a": 1, "b": 0},
            "var_to_label": ["A", "B"]
        }"#;
        let s = parse_symbols(bad.as_bytes()).expect("bad symbols parses");
        let err = verify_symbols_consistency(&s, 2).unwrap_err();
        assert!(matches!(err, ParseError::ManifestParse(_)));
    }
}
