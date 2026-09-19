// SPDX-License-Identifier: BUSL-1.1

// configflux-9pjy.3 / ADR-0039 §7 + ADR-0005 Amendment 2: the compile-time
// progress signal. `compile_model_with_progress` accepts a `ProgressSink`;
// the summary rides on `CompileResult`. Progress is a SEPARATE stream and
// never enters the byte-stable artifact — the default `compile_model` path
// wires no sink and is byte-identical to today.
use crate::{link, link_emit, link_load, link_lock};
use crate::object::{InterfaceRef, ObjectHeader};
use crate::progress::{ProgressSink, ProgressSummary, ProgressTracker, Phase};
use crate::resource_budget::ResourceBudget;
use crate::coded_error::coded_of;
use crate::{ir, Compiler};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::Path;

// Bumped 1 -> 2 (configflux-ts7z, ADR-0038 amendment Decision A.5): threading
// `override_intent` into the signed device-report payload is a wire-format change,
// so the report envelope (`SIGNED_REPORT_SCHEMA_VERSION`, which aliases this)
// advances. The version is the tamper-protected discriminator that selects the
// canonical signed shape: a report stamped below this version canonicalizes
// WITHOUT `override_intent` (so old reports still verify byte-identically), a
// report at this version canonicalizes WITH it. One monotonic schema number is
// kept across the contract surface rather than forking a report-local counter.
// Bumped 2 -> 3 (ADR-0047 §2): first-class facet declarations add `facet_index`
// to the `model_hash` preimage (via `IR_FORMAT_VERSION` 1 -> 2) and the `facets`
// namespace to the authored model, rotating `model_hash` globally this release.
// The product-contract discriminator advances in lockstep so a consumer can tell
// a facet-aware model package from a pre-facet one.
// Bumped 3 -> 4 (ADR-0054 §7): the first-class `constraints` namespace makes
// `condition` mean exactly one thing again — an inclusion selector, never a
// policy assertion (§3). A v3 request is rejected by the existing check in
// `verify_model` / `open_model` / the runtime and BOM entry points with
// E_UNSUPPORTED_SCHEMA_VERSION, the same mechanism 0.2.0 used for v2 -> v3.
//
// ZERO back-compat shims: no dual-read of v3 chunks, no `constraints`-absent
// fallback, no migration tool. The product has no external users in this window,
// and a shim written now is dead code to be maintained and tested forever.
// Coupling the semantic change to the version bump is the point — a v3 model
// that encoded a policy as a phantom component is rejected at the version check,
// before the component is ever read, so there is no window in which the same
// bytes mean two different things. `IR_FORMAT_VERSION` 2 -> 3 is the matching
// barrier on the PACKAGE side (this constant guards the REQUEST).
//
// Bumped 4 -> 5 (ADR-0057 §D7): a resolved snapshot now delivers, INSIDE each
// requiring component, the catalogue entry its requirement resolved to —
// `components.<c>.requires.<slot> = {binding, entry, fields}`. That block lives
// in `resolved_output`, which is already the largest member of both
// `resolve_hash` pre-images, so the shape of a delivered snapshot changed and
// the discriminator has to say so. Skip-if-empty keeps the addition free for a
// model that declares no requirement: such a snapshot's bytes move by this
// literal and the hashes that follow from it, and by nothing else.
//
// Same posture as 3 -> 4 and 2 -> 3: no compatibility mode, no dual-accept, no
// migration tool. A v4 request is rejected with E_UNSUPPORTED_SCHEMA_VERSION
// naming 5, before any payload is read.
pub const PRODUCT_SCHEMA_VERSION: u32 = 5;

/// registry: cause = a component's depends_on entry names a component identifier that does not exist in the model; remedy = correct the identifier or add the missing component; every depends_on target must resolve to a component the model defines
pub const E_UNKNOWN_COMPONENT_DEP: &str = "E_UNKNOWN_COMPONENT_DEP";
// ADR-0047: a facet key declared by more than one chunk (the pack-global
// at-most-one-declarer invariant), raised at ingest merge.
/// registry: cause = the same facet is declared in more than one source chunk, and a facet is a pack-global domain that may have only one declaring chunk; remedy = keep the declaration in exactly one chunk and let the others reference the facet without redeclaring it
pub const E_INGEST_DUPLICATE_FACET: &str = "E_INGEST_DUPLICATE_FACET";
// ADR-0057 §D2: a catalogue key declared by more than one chunk. Its own code
// rather than the facet one, because a duplicated TABLE and a duplicated
// DOMAIN are different authoring mistakes with different fixes.
/// registry: cause = the same catalogue is declared in more than one source chunk, and a catalogue is a pack-global table that may have only one declaring chunk; remedy = keep the table in exactly one chunk and let every other chunk bind to it rather than redeclaring it
pub const E_INGEST_DUPLICATE_CATALOGUE: &str = "E_INGEST_DUPLICATE_CATALOGUE";
// ADR-0057 §D2: a catalogue whose entries do not match its declared fields.
/// registry: cause = a catalogue declares no fields or no entries, or one of its entries omits a declared field, carries a field the catalogue never declared, gives a field a value of the wrong declared type, or gives a float field a value that is not a finite number; remedy = make every entry supply exactly the declared fields, with a finite value of each field's declared type
pub const E_CATALOGUE_INVALID: &str = "E_CATALOGUE_INVALID";
// ADR-0057 §D3: a binding that does not resolve to a catalogue, a legal
// default, or a declared derive source.
/// registry: cause = a binding names a catalogue the model never declares, defaults to an entry that catalogue does not contain, declares both a default and a derive table, or derives from something that is not a declared facet or binding or with a key or entry outside the relevant domain; remedy = read the message, which names the binding and the specific half that does not resolve, then declare the missing catalogue or facet or correct the entry name
pub const E_BINDING_INVALID: &str = "E_BINDING_INVALID";
// ADR-0057 §D4: one component requirement that does not resolve. A fault the
// author fixes on the component.
/// registry: cause = a component requirement names a binding the model never declares, or its accepts list is empty, repeats an entry, names an entry the binding's catalogue does not contain, or sits on a component whose condition does not parse; remedy = read the message, which names the component and the slot, then declare the missing binding or correct the accepted entry names
pub const E_REQUIRES_INVALID: &str = "E_REQUIRES_INVALID";
// ADR-0057 §D4: every requirement on one binding is individually legal and
// together they leave no entry standing. A fault in the model as a whole, so it
// carries its own code rather than being reported against one requirement.
/// registry: cause = the accepts lists of the components that require one binding intersect to nothing, so no catalogue entry satisfies every component that needs it; remedy = widen one of the accepts lists the message names, add an entry all of them accept, or split the components onto separate bindings if they genuinely need different entries
pub const E_BINDING_NO_ACCEPTABLE_ENTRY: &str = "E_BINDING_NO_ACCEPTABLE_ENTRY";
// ADR-0047 §3: a closed facet's declared domain is exhaustive, but a condition
// equality predicate names a value outside it.
/// registry: cause = a constraint names a facet the model never declares, or a condition or constraint names a value that is not in a closed facet's exhaustively declared domain, or a parameter binds a facet the model never declares, or a facet default that is not one of its declared values; remedy = declare the missing facet with its value domain, or drop it from the constraint; for an undeclared value, add it to the facet's declared values, mark the facet open if its domain is genuinely extensible, or correct the reference to use a declared value
pub const E_FACET_VALUE_UNDECLARED: &str = "E_FACET_VALUE_UNDECLARED";
/// registry: cause = the component dependency graph contains a cycle, so no valid build or initialization order exists; remedy = break the cycle the diagnostic traces, so that the dependency graph is acyclic
pub const E_COMPONENT_DEP_CYCLE: &str = "E_COMPONENT_DEP_CYCLE";
// RETIRED by ADR-0048: diamond dependencies are permitted (the component graph
// may be any DAG). This code is reserved and never reused — it is kept as a
// visible, never-emitted marker so the frozen diagnostic registry
// (docs/interface-contracts.md §3.4) stays a stable contract. No code path
// emits it; the constant exists only to burn the identifier under its old
// meaning.
#[allow(dead_code)]
/// registry: cause = no current code path emits this code; diamond-shaped dependencies are permitted and the component graph may be any acyclic graph; remedy = no action is needed: the code is reserved and never reused so that the registry stays a stable contract, and a diamond dependency is accepted
pub const E_COMPONENT_DEP_DIAMOND: &str = "E_COMPONENT_DEP_DIAMOND";
/// registry: cause = a source chunk could not be ingested, the model failed link and verification for a reason outside the dependency, cycle, and closed-facet families, or a declared facet name or value cannot be written as a condition-grammar token; remedy = read the wrapped message: it names the offending source and the schema or structural rule the input broke, or the facet and the value the condition grammar cannot express
pub const E_COMPILE_INPUT_INVALID: &str = "E_COMPILE_INPUT_INVALID";
/// registry: cause = the model verified successfully but its artifacts could not be written, which is almost always a permissions or filesystem problem on the output directory; remedy = choose a writable output directory and check its ownership and mount options; the diagnostic's hint names the specific filesystem condition
pub const E_COMPILE_EMIT_FAILED: &str = "E_COMPILE_EMIT_FAILED";
// configflux-p0jz.1 / ADR-0058 §D1: one object is exactly one unit, so the
// chunks of one `compile-object` call must agree on `package`. Its own family
// rather than a `COMPILE` code: the fault is in how the sources were grouped on
// the command line, not in any of them.
/// registry: cause = the chunks given to one `compile-object` call do not all declare the same `package`, and one object is exactly one unit; remedy = compile each unit into its own object and pass the others with --interface
pub const E_OBJECT_UNIT_MISMATCH: &str = "E_OBJECT_UNIT_MISMATCH";
// configflux-p0jz.2 / ADR-0058 §D4: the link-stage family. Their own family
// rather than COMPILE codes because each one is a fault BETWEEN objects — a
// linked set that is wrong, not a chunk that is — and because the fix is always
// to change which objects were linked or to recompile one of them.
/// registry: cause = two linked objects declare the same unit, and a unit names exactly one object in a link; remedy = link one object per unit: drop the stale copy, or recompile the unit once
pub const E_LINK_DUPLICATE_UNIT: &str = "E_LINK_DUPLICATE_UNIT";
/// registry: cause = two linked objects export the same entity id, and an id names one declaration across the whole linked set; remedy = rename one of the two declarations, or link only the unit that owns the id
pub const E_LINK_DUPLICATE_ID: &str = "E_LINK_DUPLICATE_ID";
/// registry: cause = a linked unit references an entity that no linked object declares, so the object that would provide it was not passed to the link; remedy = link the object whose unit declares it, or drop the reference
pub const E_LINK_UNRESOLVED_IMPORT: &str = "E_LINK_UNRESOLVED_IMPORT";
/// registry: cause = a linked unit was compiled against a different version of an interface object than the one being linked, so what it was checked against is not what it will run with; remedy = recompile the dependent unit against this interface object, or link the interface object it was compiled against
pub const E_LINK_INTERFACE_MISMATCH: &str = "E_LINK_INTERFACE_MISMATCH";
/// registry: cause = a linked object holds a chunk file that is missing, unreadable, does not match the header that names it, or no longer hashes to the name it is filed under, or a header that does not match the chunk files it names, so the object cannot be trusted to carry the unit its header describes; remedy = recompile the object with `compile-object`; a linked object is a build product and is never repaired by hand
pub const E_LINK_OBJECT_CORRUPT: &str = "E_LINK_OBJECT_CORRUPT";
// configflux-p0jz.3 / ADR-0058 §D5: the lockfile family. A lock pins, per unit,
// the object hash an integration expects; the linker CHECKS those pins and
// never fetches anything. The three codes separate the three questions a lock
// can answer badly: the file is not a lock, the build is not the reviewed one,
// and the reviewed set is not the built one.
/// registry: cause = a lockfile pins a unit at an object hash that is not the one being linked, a linked unit is not pinned at all, or `--write-lock` would overwrite a lockfile whose contents differ from the one this link produces; remedy = link the objects the lock pins, or renew the pins with `link --write-lock --force-lock` once the change has been reviewed
pub const E_LINK_LOCK_MISMATCH: &str = "E_LINK_LOCK_MISMATCH";
/// registry: cause = a lockfile pins a unit that no linked object provides, so the set being built is not the set that was reviewed; remedy = link the object the lock pins, drop the entry with `link --write-lock --force-lock`, or pass `--lock-allow-extra` when the subset link is deliberate
pub const E_LINK_LOCK_UNLINKED: &str = "E_LINK_LOCK_UNLINKED";
/// registry: cause = the file given to `--lock` could not be read, is not a lockfile, carries a key this version does not know, is at an unsupported `schema_version`, or is keyed by something that is not a unit name; remedy = point `--lock` at a lockfile written by `link --write-lock`, or fix the key the message names
pub const E_LINK_LOCK_INVALID: &str = "E_LINK_LOCK_INVALID";
/// registry: cause = the request's schema_version is not the version this build implements, and older versions are rejected rather than silently adapted; remedy = set schema_version to the version this binary reports, or use a binary built for the version your caller targets
pub const E_UNSUPPORTED_SCHEMA_VERSION: &str = "E_UNSUPPORTED_SCHEMA_VERSION";
/// registry: cause = the inspect query names a component identifier the model does not define; remedy = check the identifier for typos and list the model's components to find the one you meant
pub const E_INSPECT_UNKNOWN_COMPONENT: &str = "E_INSPECT_UNKNOWN_COMPONENT";
/// registry: cause = the inspect query names a definition identifier the model does not define; remedy = check the identifier for typos and list the model's definitions to find the one you meant
pub const E_INSPECT_UNKNOWN_DEFINITION: &str = "E_INSPECT_UNKNOWN_DEFINITION";
/// registry: cause = the inspect query names an artifact identifier the model does not define; remedy = check the identifier for typos and list the model's artifacts to find the one you meant
pub const E_INSPECT_UNKNOWN_ARTIFACT: &str = "E_INSPECT_UNKNOWN_ARTIFACT";
/// registry: cause = the component in the inspect query exists, but it declares no parameter by the requested key; remedy = inspect the component first to list the parameter keys it actually declares
pub const E_INSPECT_UNKNOWN_PARAMETER: &str = "E_INSPECT_UNKNOWN_PARAMETER";
/// registry: cause = the inspect scope names a component that does not exist, names a component that is not a platform where a platform was required, or matches no components at all; remedy = use a scope whose component exists and has the expected type; a platform selector must name a component declared as a platform
pub const E_INSPECT_UNKNOWN_SCOPE: &str = "E_INSPECT_UNKNOWN_SCOPE";
/// registry: cause = the inspect query is malformed: an identifier is not a valid snake-case name, the scope string does not parse, or a parameter's metadata could not be materialized; remedy = correct the identifier or scope syntax; if the failure is in materializing parameter metadata, check the parameter's definition chain for a broken link
pub const E_INSPECT_QUERY_INVALID: &str = "E_INSPECT_QUERY_INVALID";

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Ok,
    Error,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyCheckStatus {
    Pass,
    Fail,
    Warn,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceManifestEntry {
    pub source_id: String,
    pub inline_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyModelRequest {
    pub schema_version: u32,
    pub source_manifest: Vec<SourceManifestEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileModelRequest {
    pub schema_version: u32,
    pub source_manifest: Vec<SourceManifestEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<String>,
    /// Target maximum number of distinct variables per BDD partition —
    /// configflux-vmlb / ADR-0012 §2. When `None`, the partitioner
    /// collapses to a single partition (`usize::MAX` default,
    /// preserving FAMA/SPLOT fixtures' single-partition layout). The
    /// CLI surfaces this via `--cluster-size`; the field is plumbed
    /// here so the compile API surface is self-describing for
    /// downstream callers (`configflux-dpst` 4 GB bench,
    /// `configflux-4fu0` fixture rotation). As of configflux-9hi2 the
    /// `compile_model` product path emits the `.ccm` artifact itself when
    /// `output_dir` is set, and this field is the partitioning input for
    /// that emission (it also remains the canonical wiring point for
    /// out-of-band emitters such as `tools/gen_synthetic`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_size: Option<usize>,
    /// Soft resource budget for the compile — configflux-9pjy.2 / ADR-0039.
    /// `None` (the default) is byte-for-byte identical to today: no memo
    /// cap is applied and no `cluster_size` is derived. When set, the
    /// budget derives an apply-memo cap (byte-neutral cache lever) and,
    /// if the projected unique table would exceed the budget, a
    /// `cluster_size` (explicit `cluster_size` above always wins, per
    /// ADR-0012 Amendment 1). `serde(default)` keeps requests serialized
    /// before this field existed deserializing unchanged, and
    /// `skip_serializing_if` keeps an unset budget out of the wire form
    /// entirely (mirrors `cluster_size`).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub budget: Option<ResourceBudget>,
    /// ADR-0044 D1 (`configflux-pq2w.1`): when `true`, the emitted
    /// provenance sidecars carry a wall-clock `stamped_at`. Default `false`
    /// keeps compile output byte-stable (same inputs → same bytes, sidecar
    /// included). `serde(default)` keeps pre-field requests deserializing
    /// unchanged; `skip_serializing_if` keeps the clean default out of the
    /// canonical wire form (mirrors `cluster_size`/`budget`). The sidecar is
    /// non-hashed, so this never touches any artifact hash preimage.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stamp_time: bool,
}

/// `skip_serializing_if` predicate: keep a `false` flag out of the canonical
/// wire form (serde needs a `&bool -> bool` fn path).
fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiagnosticsReport {
    pub schema_version: u32,
    pub diagnostics: Vec<Diagnostic>,
    pub error_count: u32,
    pub warning_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyCheckResult {
    pub check_id: String,
    pub status: VerifyCheckStatus,
    pub summary: String,
    pub diagnostic_codes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifyReport {
    pub schema_version: u32,
    /// The CMP model identity of the model this report describes, present
    /// exactly when an index was built for it (ADR-0056 Amendment 2 D2).
    ///
    /// Standalone `verify` reports the hash `compile` emits for the same
    /// sources, because both derive it from an index built by the same staging
    /// (`stage_in_memory`). It is ABSENT — not a placeholder — on every failing
    /// path of `verify` and on every error path of `compile` and `link`, where
    /// no model was indexed and there is therefore no identity to report.
    ///
    /// It used to be seeded with the source-manifest digest, which made one
    /// name denote two different functions depending on which command ran:
    /// `compile` overwrote it with the real identity on success, and every
    /// other path published the digest under the identity's name. That digest
    /// is `InspectionResult.source_digest` (ADR-0056 §9.2) and this field never
    /// repeats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    pub status: OperationStatus,
    pub error_count: u32,
    pub warning_count: u32,
    pub checks: Vec<VerifyCheckResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileStats {
    pub source_count: u32,
    pub chunk_count: u32,
    pub definition_count: u32,
    pub component_count: u32,
    pub artifact_count: u32,
}

/// Next-run advisory that the effective `cluster_size` was too large for
/// the soft budget (configflux-9pjy.4 / ADR-0039 §5). Recorded only when a
/// partition's post-build RSS approached the budget. The compiler does
/// **not** re-partition mid-run — the partition layout is part of the
/// byte-stable pre-image (ADR-0012 determinism) — so this is purely a hint
/// for the operator's next invocation (lower `--cluster-size` or raise
/// `--max-rss-mb`). Metadata only; never enters the artifact bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterSizeAdvisory {
    /// The effective `cluster_size` this run used (the value to reduce next
    /// time). `u64::MAX` denotes single-partition collapse (no
    /// `--cluster-size`), i.e. partitioning should be introduced.
    pub effective_cluster_size: u64,
    /// Human-readable next-run guidance.
    pub message: String,
}

/// Soft-budget adaptation report for a compile (configflux-9pjy.4 /
/// ADR-0039 §5). Populated on [`CompileResult`] only when a
/// [`ResourceBudget`] with an `max_rss_mb` drove the adaptive path;
/// `None`/absent otherwise so the default wire form is unchanged
/// (`skip_serializing_if`). Surfaces how far the byte-neutral live
/// memo-cap shrink pushed the cache and the cross-partition advisory.
/// This is a SEPARATE summary on the API result — it never enters
/// `ccm.manifest.json`, `ccm.symbols.json`, `ccm.bdd.bin`, or
/// `partition-manifest.json` (ADR-0005 §6 / Amendment 2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReport {
    /// Total live memo-cap halvings across all partitions (`0` when the
    /// budget was never approached).
    pub memo_shrink_count: u32,
    /// The smallest final memo cap any partition reached (the deepest the
    /// budget pushed the cache), or `None` when no shrink fired.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub final_memo_cap: Option<u64>,
    /// The cross-partition "cluster_size too large" advisory, present only
    /// when triggered (ADR-0039 §5). Absent by default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub cluster_size_advisory: Option<ClusterSizeAdvisory>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompileResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled_model_package_ref: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub stats: CompileStats,
    pub verify_report: VerifyReport,
    /// Compile-time progress summary (configflux-9pjy.3 / ADR-0039 §7).
    /// Populated only when a [`ProgressSink`] is wired
    /// (`compile_model_with_progress`); `None` for the default
    /// `compile_model` path, which keeps the wire form byte-identical to
    /// today (the `skip_serializing_if` drops the absent field entirely).
    /// Progress is a SEPARATE stream and never enters the byte-stable
    /// artifact (ADR-0005 Amendment 2); this summary lives on the API
    /// result, not in any `ccm.*` file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub progress_summary: Option<ProgressSummary>,
    /// Soft-budget adaptation report (configflux-9pjy.4 / ADR-0039 §5).
    /// Populated only when a [`ResourceBudget`] with `max_rss_mb` drove the
    /// adaptive path; `None`/absent otherwise so the default (unbudgeted)
    /// wire form is byte-identical to today. Carries the live memo-cap
    /// shrink count / final cap and the cross-partition advisory. A
    /// SEPARATE summary on the result — never in any `ccm.*` artifact file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub budget_report: Option<BudgetReport>,
    /// Tool-identity stamp (ADR-0044 D1 / `configflux-pq2w.1`). The
    /// workspace `tool_version` (from `/VERSION`), populated by the product
    /// compile path so any consumer holding this envelope can answer "which
    /// tool version produced this". Additive + optional (serde
    /// `skip_serializing_if`), mirroring `budget_report`: it carries
    /// identity on the side-channel and never enters a hashed artifact byte,
    /// and adding it does NOT bump `schema_version`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_version: Option<String>,
    /// The objects this package was linked from (ADR-0058 §D4), each named by
    /// its unit and its `object_hash`, unit-ascending.
    ///
    /// Populated by [`link_model`] and EMPTY on the `compile` path, which is
    /// not an omission. `compile` groups its `--source` chunks into units and
    /// links them in memory (§D8); those objects are never written, and they
    /// record no `interfaces`, so their `object_hash` would be the identity of
    /// an artifact nobody holds and that `compile-object` — which does record
    /// its interfaces — would not reproduce. Reporting it would invite a
    /// comparison that cannot be made.
    ///
    /// `skip_serializing_if` therefore also keeps the `compile` envelope
    /// byte-identical to the one before this field existed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<InterfaceRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "query_type", rename_all = "snake_case")]
pub enum InspectQuery {
    Summary,
    Component {
        component_id: String,
    },
    Definition {
        definition_id: String,
    },
    Artifact {
        artifact_id: String,
    },
    Parameter {
        component_id: String,
        param_key: String,
    },
    ScopedStats {
        scope: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectModelRequest {
    pub schema_version: u32,
    pub source_manifest: Vec<SourceManifestEntry>,
    pub query: InspectQuery,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectionSummary {
    pub source_count: u32,
    pub definition_count: u32,
    pub component_count: u32,
    pub artifact_count: u32,
    pub definition_ids: Vec<String>,
    pub component_ids: Vec<String>,
    pub artifact_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "entity_kind", rename_all = "snake_case")]
pub enum InspectionItem {
    Component {
        component_id: String,
        component_type: Option<String>,
        condition: Option<String>,
        depends_on: Vec<String>,
        /// What this component needs from the model's bindings, slot ->
        /// requirement (ADR-0057 §D4/§D7). Reported in the authored shape,
        /// because that is the shape the author has to change. It sits beside
        /// `depends_on` and is deliberately NOT folded into it: a requirement
        /// names a shared CHOICE, not another component.
        ///
        /// Skip-if-empty, so `inspect component` output for a model that
        /// declares no requirement is byte-identical to what it was before.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        requires: BTreeMap<String, crate::schema::Requirement>,
        param_count: u32,
        param_keys: Vec<String>,
    },
    Definition {
        definition_id: String,
        param_type: Option<String>,
        inherits: Option<String>,
        has_value: bool,
        override_count: u32,
    },
    Artifact {
        artifact_id: String,
        name: String,
        version: Option<String>,
        hash: Option<String>,
        source: Option<String>,
        target: Option<String>,
    },
    Parameter {
        component_id: String,
        param_key: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        inherits: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        r#type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<crate::schema::Value>,
        /// The facet this parameter is the declared handle for (ADR-0064 D1),
        /// when the model declared one.
        ///
        /// A handle authors no `value` — the declaration rules refuse one — so
        /// without this the compile-time read shows a parameter with no value
        /// and no account of where its value comes from, while the runtime read
        /// of the same parameter names the facet. Serialized exactly as
        /// `ResolvedParameter::facet`, so the two reads agree.
        ///
        /// `skip_serializing_if` keeps an UNBOUND parameter byte-identical to
        /// what it was before the field existed, which is what leaves the
        /// committed inspect goldens untouched.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        facet: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        unit: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lifecycle: Option<crate::schema::Lifecycle>,
        #[serde(skip_serializing_if = "Option::is_none")]
        safety: Option<crate::schema::SafetyLevel>,
        #[serde(skip_serializing_if = "Option::is_none")]
        access: Option<crate::schema::Role>,
        #[serde(skip_serializing_if = "Option::is_none")]
        req_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        doc: Option<String>,
        override_count: u32,
        override_conditions: Vec<String>,
        candidate_artifact_ids: Vec<String>,
    },
    ScopedStats {
        scope: String,
        scope_roots: Vec<String>,
        component_count: u32,
        parameter_count: u32,
        artifact_count: u32,
        component_ids: Vec<String>,
        artifact_ids: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InspectionResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    /// The source-manifest digest (ADR-0056 §9.2). Deliberately not
    /// `model_hash`: `inspect_model` never emits a package, so this can never
    /// hold the CMP identity that `model_hash` denotes on the compile path.
    pub source_digest: String,
    pub query: InspectQuery,
    pub summary: InspectionSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<InspectionItem>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

pub fn verify_model(request: VerifyModelRequest) -> VerifyReport {
    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        let diagnostic = Diagnostic {
            code: E_UNSUPPORTED_SCHEMA_VERSION.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unsupported schema_version {} (expected {})",
                request.schema_version, PRODUCT_SCHEMA_VERSION
            ),
            source_id: None,
            entity_path: None,
            hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
        };
        return verify_report_with_failures(
            None,
            "graph_integrity",
            "Unsupported schema version",
            vec![diagnostic],
        );
    }

    let mut compiler = Compiler::new();
    for source in &request.source_manifest {
        if let Err(err) =
            compiler.add_chunk_auto(source.source_id.clone(), &source.inline_content)
        {
            return verify_report_with_failures(
                None,
                "graph_integrity",
                "Model ingestion failed",
                vec![map_compile_input_error(
                    &err,
                    Some(source.source_id.clone()),
                )],
            );
        }
    }

    let mut report = verify_compiler(&compiler);
    if report.status != OperationStatus::Ok {
        return report;
    }

    // ADR-0056 Amendment 2 D1: a model that verified HAS an identity, and it is
    // the one `compile` would emit for the same sources. Build the index the
    // way `compile` builds it — in memory, nothing written — and report it.
    //
    // A fault here is reported through the shape `compile` uses for the same
    // fault (D4). On a complete model it cannot happen: `link_and_verify` just
    // passed, and the header stage can only see faults those checks already
    // cover when every unit is present. The arm exists because the staging
    // returns a `Result` and swallowing it would report an identity-free `ok`.
    match stage_in_memory(&compiler) {
        Ok((_, _, index)) => report.model_hash = Some(index.config_hash),
        Err(diagnostic) => {
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            report =
                verify_report_with_failures(None, "graph_integrity", &summary, vec![diagnostic]);
        }
    }
    report
}

/// The in-memory link staging `compile` and `verify` share: group the ingested
/// chunks into one object per unit, run the linker's header stage over them,
/// render the chunk bytes, and build the index those chunks declare.
///
/// One function rather than two call sequences, because the index it returns is
/// the CMP model identity (`ir::CmpManifest::from_index`). A `verify` that
/// staged the model even slightly differently would report a hash `compile`
/// never emits, which is exactly the class of defect ADR-0056 Amendment 2
/// closes — so the two commands share the derivation rather than agreeing about
/// it.
///
/// Nothing here touches the filesystem: `compile` hands the index straight to
/// [`link_emit::write_package`], and `verify` reports it and stops.
fn stage_in_memory(
    compiler: &Compiler,
) -> Result<
    (
        crate::interface_summary::MergedSummary,
        Vec<link_emit::LinkChunk>,
        ir::IrIndex,
    ),
    Diagnostic,
> {
    let summaries = compiler.interface_summaries();
    let headers = link::in_memory_headers(compiler.source_chunks(), &summaries);
    let merged = link::link_stage_headers(&headers)?;
    let chunks =
        link_emit::link_chunks_of(compiler, &summaries).map_err(|err| emit_diagnostic(&err))?;
    let index = link_emit::build_package_index(&chunks).map_err(|err| emit_diagnostic(&err))?;
    Ok((merged, chunks, index))
}

/// One `link` invocation (ADR-0058 §D4).
///
/// The CCM and resource knobs are exactly `compile`'s, and mean exactly the
/// same thing: the constraint model is a link product, so the flags that shape
/// it belong to the step that builds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkModelRequest {
    pub schema_version: u32,
    /// The object directories to link, in any order. Argument order cannot
    /// reach a byte: the linker visits objects by unit name.
    pub object_dirs: Vec<String>,
    /// The package directory to write. Unlike `compile`'s, this is required —
    /// a link with nothing to emit is a verification, and `verify` is that.
    pub output_dir: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub budget: Option<ResourceBudget>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub stamp_time: bool,
    /// The lockfile whose pins this link must satisfy (ADR-0058 §D5). Checked
    /// at the top of stage 1, before any other question is asked of the
    /// linked set: a link that is not the one the integration reviewed is
    /// refused before its structure is even considered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lock_path: Option<String>,
    /// Waive the "every lock entry is linked" half of the check, for a
    /// deliberate subset link — an agent linking only the closure it needs.
    /// The "every linked object is pinned" half always holds.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lock_allow_extra: bool,
    /// The lockfile to write from the linked objects after a successful link.
    /// May name the same path as `lock_path`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write_lock_path: Option<String>,
    /// Per-unit `source` notes for the written lock. Informational: the
    /// product records what it is given and reads it for no purpose, because
    /// nothing here ever fetches an object. An entry for a unit that was not
    /// linked has nothing to annotate and is dropped.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub lock_sources: BTreeMap<String, String>,
    /// Overwrite an existing lockfile that differs from the one this link
    /// would write.
    #[serde(default, skip_serializing_if = "is_false")]
    pub force_lock: bool,
}

/// Link a set of objects into the package `compile` produces (ADR-0058 §D4).
///
/// The three stages in order, and nothing is written under `output_dir` unless
/// all three pass: the header-only graph checks, the constraint model built
/// from those headers, and the emit — which is the first step that opens a
/// chunk file.
///
/// Returns the same [`CompileResult`] `compile` does, because it produces the
/// same thing. The only field that differs is `objects`, which names what was
/// linked.
pub fn link_model(request: LinkModelRequest) -> CompileResult {
    // The default path wires no progress sink, mirroring `compile_model`:
    // byte-identical to a link without the flag, and `progress_summary` stays
    // absent from the wire form (ADR-0005 Amendment 2).
    link_model_with_progress(request, None)
}

/// As [`link_model`], plus an optional progress sink (ADR-0039 §7).
///
/// `link` takes the CCM and resource flags exactly as `compile` does (ADR-0058
/// §D4 item 1), and `--progress` is one of them — the constraint model is a LINK
/// product, so this is the step whose var-order → apply → serialize band an
/// operator has reason to watch. The stages before it are header-sized and
/// report as three completed phases, the same three `compile` marks before its
/// own emit.
///
/// Progress is a SEPARATE stream: neither the sink nor the summary reaches
/// `ccm.manifest.json`, `ccm.symbols.json`, `ccm.bdd.bin`,
/// `partition-manifest.json`, or any package file, so a watched link and an
/// unwatched one write the same bytes (`//compiler:link_progress_test`). That
/// property is what keeps the §D8 oracle true for a link under observation.
pub fn link_model_with_progress(
    request: LinkModelRequest,
    sink: Option<&dyn ProgressSink>,
) -> CompileResult {
    let tool_version = crate::provenance_sidecar::tool_version().to_string();
    // A link has no `--source` manifest to digest, so there is no pre-emit
    // identity at all: the real `model_hash` arrives with the index. The verify
    // report omits the field entirely (ADR-0056 Amendment 2 D2) rather than
    // publishing a placeholder that invites being compared; the envelope's own
    // `model_hash` keeps the empty string it has always carried, because
    // `CompileResult.model_hash` is untouched by that amendment (D3).
    let empty_hash = String::new();
    let fail = |summary: &str, diagnostic: Diagnostic| {
        let report =
            verify_report_with_failures(None, "graph_integrity", summary, vec![diagnostic]);
        compile_error(empty_hash.clone(), link_stats(0, 0), report, &tool_version)
    };

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        return fail(
            "Unsupported schema version",
            Diagnostic {
                code: E_UNSUPPORTED_SCHEMA_VERSION.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!(
                    "Unsupported schema_version {} (expected {})",
                    request.schema_version, PRODUCT_SCHEMA_VERSION
                ),
                source_id: None,
                entity_path: None,
                hint: Some(format!("Set request.schema_version to {PRODUCT_SCHEMA_VERSION}")),
            },
        );
    }

    let headers = match link_load::read_headers(&request.object_dirs) {
        Ok(headers) => headers,
        Err(diagnostic) => {
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            return fail(&summary, diagnostic);
        }
    };
    let objects: Vec<InterfaceRef> = {
        let mut refs: Vec<InterfaceRef> = headers.iter().map(ObjectHeader::as_interface_ref).collect();
        refs.sort();
        refs
    };

    // The lock, first (ADR-0058 §D5). "Before anything else" is deliberate: a
    // set that is not the one the integration reviewed should be refused for
    // being the wrong set, not for whatever the wrong set happens to also be
    // wrong about. It is the cheapest check in the link — a small file and one
    // comparison per object — and it needs only the headers already read.
    if let Some(lock_path) = request.lock_path.as_deref() {
        let checked = link_lock::read_lock(lock_path).and_then(|lock| {
            link_lock::check_lock(&lock, &headers, request.lock_allow_extra, lock_path)
        });
        if let Err(diagnostic) = checked {
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            return fail(&summary, diagnostic);
        }
    }

    // Stage 1. No chunk file has been opened yet, and none is opened if this
    // fails — the property `link_diagnostics`'s unreadable-placeholder case
    // pins (ADR-0058 §D4).
    let merged = match link::link_stage_headers(&headers) {
        Ok(merged) => merged,
        Err(diagnostic) => {
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            return fail(&summary, diagnostic);
        }
    };

    // Stage 3's read, and the complete-model checks over what it read. Both run
    // before anything is written.
    let model = match link_load::load_chunks(&request.object_dirs, &headers) {
        Ok(model) => model,
        Err(diagnostic) => {
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            return fail(&summary, diagnostic);
        }
    };
    if let Err(err) = crate::compiler_core::verify_complete_model(&model.repository, &model.summaries)
    {
        let diagnostic = map_graph_error(&err);
        let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
        return fail(&summary, diagnostic);
    }

    let stats = link_stats(objects.len() as u32, model.chunks.len() as u32);
    let output_dir = Path::new(&request.output_dir);
    let written = link_emit::build_package_index(&model.chunks).and_then(|index| {
        link_emit::write_package(output_dir, &model.chunks, &index).map(|()| index)
    });
    let index = match written {
        Ok(index) => index,
        Err(err) => {
            let diagnostic = emit_diagnostic(&err);
            let report = verify_report_with_failures(
                None,
                "graph_integrity",
                "Failed to emit compiled model package",
                vec![diagnostic],
            );
            return compile_error(empty_hash, stats, report, &tool_version);
        }
    };

    let model_hash = index.config_hash;
    // The header stages and the package write are done by the time the emitter
    // takes over, so mark those three phase boundaries before the var-order →
    // apply → serialize band, exactly as `compile_model_with_progress` does.
    // The tracker exists only when a sink is wired; with none, the emit takes
    // the byte-identical default path.
    let mut tracker = sink.map(ProgressTracker::new);
    if let Some(t) = tracker.as_mut() {
        t.emit_phase_complete(Phase::Ingest, 0);
        t.emit_phase_complete(Phase::Merge, 0);
        t.emit_phase_complete(Phase::Link, 0);
    }
    let ccm_model = link::link_stage_model(&model_hash, &merged);
    let ccm = link_emit::emit_ccm_sibling(
        output_dir,
        &ccm_model,
        request.cluster_size,
        request.budget.as_ref(),
        tracker.as_mut(),
    );
    let (ccm_dir, outcome) = match ccm {
        Ok(pair) => pair,
        Err(err) => {
            let report = ccm_emit_report(None, &err, &ccm_model.facet_domains);
            return compile_error(model_hash, stats, report, &tool_version);
        }
    };
    if let Err(err) = write_compile_provenance(output_dir, &ccm_dir, request.stamp_time) {
        let report = verify_report_with_failures(
            None,
            "graph_integrity",
            "Failed to write provenance sidecar",
            vec![emit_diagnostic(&err)],
        );
        return compile_error(model_hash, stats, report, &tool_version);
    }

    // The lock is written from a link that SUCCEEDED (ADR-0058 §D5), so it
    // records a set that was actually built rather than one that was merely
    // asked for. A refusal here — an existing lock this link would change —
    // therefore leaves the package under `output_dir` in place and still exits
    // non-zero: the link was fine, and it is the pin that was not renewed.
    if let Some(write_path) = request.write_lock_path.as_deref() {
        if let Err(diagnostic) = link_lock::write_lock(
            write_path,
            &headers,
            &request.lock_sources,
            request.force_lock,
        ) {
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            let report =
                verify_report_with_failures(None, "graph_integrity", &summary, vec![diagnostic]);
            return compile_error(model_hash, stats, report, &tool_version);
        }
    }

    // The terminal Serialize completion (overall pct == 1.0) and the summary.
    // `total_clauses` is the emitter's to know; `finish` needs it only to stamp
    // the final event's observational counters, and the pct still terminates at
    // exactly 1.0 because the phase weights are fixed.
    let progress_summary = tracker.map(|t| t.finish(0));

    CompileResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash: model_hash.clone(),
        compiled_model_package_ref: Some(
            output_dir
                .join(ir::CMP_DEFAULT_MANIFEST_FILENAME)
                .to_string_lossy()
                .into_owned(),
        ),
        diagnostics_ref: None,
        stats,
        verify_report: verify_report_ok(Some(model_hash)),
        progress_summary,
        budget_report: budget_report_from_outcome(request.budget.as_ref(), &outcome),
        tool_version: Some(tool_version),
        objects,
    }
}

/// A link's [`CompileStats`], with the counts a link can actually answer.
///
/// `source_count` is the number of OBJECTS linked, because that is what a link
/// was given; `chunk_count` is the chunks they carry. The three entity counts
/// stay zero rather than being recomputed from the linked model: they describe
/// a merged authoring view that a link does not build, and a number that looked
/// like `compile`'s but was derived differently would be worse than none.
fn link_stats(object_count: u32, chunk_count: u32) -> CompileStats {
    CompileStats {
        source_count: object_count,
        chunk_count,
        definition_count: 0,
        component_count: 0,
        artifact_count: 0,
    }
}

pub fn compile_model(request: CompileModelRequest) -> CompileResult {
    // The default path wires no progress sink: byte-identical to today
    // and `progress_summary` stays absent from the wire form (ADR-0005
    // Amendment 2).
    compile_model_with_progress(request, None)
}

/// As [`compile_model`], plus an optional compile-time progress sink
/// (configflux-9pjy.3 / ADR-0039 §7). When `sink` is `Some`, the compile
/// emits a weighted-phase progress stream to it (ingest → merge → link →
/// var-order → BDD apply loop → serialize) and the returned
/// [`CompileResult::progress_summary`] is populated; when `None`, this is
/// the exact default compile — byte-for-byte identical output, no summary.
///
/// Progress is a SEPARATE stream (ADR-0005 Amendment 2): the sink and the
/// summary never enter `ccm.manifest.json`, `ccm.symbols.json`,
/// `ccm.bdd.bin`, or `partition-manifest.json`.
pub fn compile_model_with_progress(
    request: CompileModelRequest,
    sink: Option<&dyn ProgressSink>,
) -> CompileResult {
    let source_count = request.source_manifest.len() as u32;
    let mut compiler = Compiler::new();
    let mut chunk_count = 0_u32;
    let mut model_hash = hash_sources(&request.source_manifest);
    // ADR-0044 D1: stamp every result envelope with the workspace version so
    // identity rides the side-channel (never a hashed byte). Cheap `&str`.
    let tool_version_stamp = crate::provenance_sidecar::tool_version().to_string();

    if request.schema_version != PRODUCT_SCHEMA_VERSION {
        let diagnostic = Diagnostic {
            code: E_UNSUPPORTED_SCHEMA_VERSION.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unsupported schema_version {} (expected {})",
                request.schema_version, PRODUCT_SCHEMA_VERSION
            ),
            source_id: None,
            entity_path: None,
            hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
        };
        let verify_report = verify_report_with_failures(
            None,
            "graph_integrity",
            "Unsupported schema version",
            vec![diagnostic],
        );
        return CompileResult {
            schema_version: PRODUCT_SCHEMA_VERSION,
            status: OperationStatus::Error,
            model_hash,
            compiled_model_package_ref: None,
            diagnostics_ref: None,
            stats: compile_stats(source_count, chunk_count, &compiler),
            verify_report,
            progress_summary: None,
            budget_report: None,
            tool_version: Some(tool_version_stamp.clone()),
            objects: Vec::new(),
        };
    }

    for source in &request.source_manifest {
        match compiler.add_chunk_auto(source.source_id.clone(), &source.inline_content) {
            Ok(()) => {
                chunk_count += 1;
            }
            Err(err) => {
                let verify_report = verify_report_with_failures(
                    None,
                    "graph_integrity",
                    "Model ingestion failed",
                    vec![map_compile_input_error(
                        &err,
                        Some(source.source_id.clone()),
                    )],
                );
                return CompileResult {
                    schema_version: PRODUCT_SCHEMA_VERSION,
                    status: OperationStatus::Error,
                    model_hash,
                    compiled_model_package_ref: None,
                    diagnostics_ref: None,
                    stats: compile_stats(source_count, chunk_count, &compiler),
                    verify_report,
                    progress_summary: None,
                    budget_report: None,
                    tool_version: Some(tool_version_stamp.clone()),
                    objects: Vec::new(),
                };
            }
        }
    }

    let mut verify_report = verify_compiler(&compiler);
    if verify_report.status == OperationStatus::Error {
        return CompileResult {
            schema_version: PRODUCT_SCHEMA_VERSION,
            status: OperationStatus::Error,
            model_hash,
            compiled_model_package_ref: None,
            diagnostics_ref: None,
            stats: compile_stats(source_count, chunk_count, &compiler),
            verify_report,
            progress_summary: None,
            budget_report: None,
            tool_version: Some(tool_version_stamp.clone()),
            objects: Vec::new(),
        };
    }

    let mut compiled_model_package_ref = None;
    let mut progress_summary = None;
    // configflux-9pjy.4 / ADR-0039 §5: the soft-budget adaptation report,
    // populated from the emitter's `EmitBudgetOutcome` when a budget with
    // an RSS target drove the adaptive path. Stays `None` (absent from the
    // wire form) on the unbudgeted default path.
    let mut budget_report = None;
    if let Some(output_dir) = request.output_dir {
        // ADR-0058 §D8: group the ingested chunks into one in-memory object per
        // unit and run the linker's stages over them. The complete-model checks
        // already ran above (`verify_compiler`), so stage 1 can only report a
        // fault those checks cannot see — which, with every unit present, is
        // none. This is what makes `compile` and `link` one code path without
        // moving any diagnostic off the code it has always carried.
        let staged = stage_in_memory(&compiler).and_then(|(merged, chunks, index)| {
            link_emit::write_package(Path::new(&output_dir), &chunks, &index)
                .map(|()| (merged, index))
                .map_err(|err| emit_diagnostic(&err))
        });
        match staged {
            Ok((merged, index)) => {
                model_hash = index.config_hash;
                compiled_model_package_ref = Some(
                    Path::new(&output_dir)
                        .join(ir::CMP_DEFAULT_MANIFEST_FILENAME)
                        .to_string_lossy()
                        .into_owned(),
                );
                verify_report.model_hash = Some(model_hash.clone());

                // configflux-9pjy.3: ingest, merge, and link are complete by
                // the time we reach the .ccm emit, so mark those phase
                // boundaries on the tracker before the emitter drives the
                // var-order → apply → serialize band. The tracker is built
                // only when a sink is wired; with no sink the emit takes the
                // exact byte-identical default path.
                let mut tracker = sink.map(ProgressTracker::new);
                if let Some(t) = tracker.as_mut() {
                    t.emit_phase_complete(Phase::Ingest, 0);
                    t.emit_phase_complete(Phase::Merge, 0);
                    t.emit_phase_complete(Phase::Link, 0);
                }

                // configflux-9hi2: additively emit the v2 multi-part `.ccm`
                // sibling so the solver has a real artifact to load (ADR-0005
                // Amendment 1, ADR-0017 §2); bound to the CMP model_hash (§9).
                // configflux-p0jz.2 / ADR-0058 §D4 stage 2: the model is built
                // from the merged object headers, in the canonical clause
                // order, by the same function the linker calls.
                let model = link::link_stage_model(&model_hash, &merged);
                match link_emit::emit_ccm_sibling(
                    Path::new(&output_dir),
                    &model,
                    request.cluster_size,
                    request.budget.as_ref(),
                    tracker.as_mut(),
                ) {
                    Ok((ccm_dir, outcome)) => {
                        budget_report =
                            budget_report_from_outcome(request.budget.as_ref(), &outcome);
                        // ADR-0044 D1 (configflux-pq2w.1): write the
                        // deterministic, non-hashed provenance sidecars next
                        // to the two file-writing artifact sets — the CMP dir
                        // and its sibling CCM dir. Atomic temp+rename; no
                        // wall-clock unless `stamp_time`. This runs after both
                        // emits succeeded, so the recorded content hashes are
                        // over the final artifact bytes.
                        if let Err(err) = write_compile_provenance(
                            Path::new(&output_dir),
                            &ccm_dir,
                            request.stamp_time,
                        ) {
                            verify_report = verify_report_with_failures(
                                None,
                                "graph_integrity",
                                "Failed to write provenance sidecar",
                                vec![emit_diagnostic(&err)],
                            );
                            return compile_error(
                                model_hash,
                                compile_stats(source_count, chunk_count, &compiler),
                                verify_report,
                                &tool_version_stamp,
                            );
                        }
                    }
                    Err(err) => {
                        verify_report = ccm_emit_report(None, &err, &model.facet_domains);
                        return compile_error(
                            model_hash,
                            compile_stats(source_count, chunk_count, &compiler),
                            verify_report,
                            &tool_version_stamp,
                        );
                    }
                }

                // configflux-9pjy.3: emit the terminal Serialize completion
                // (overall pct == 1.0) and capture the summary. `total_clauses`
                // is unknown here (it lives in the emitter), but the emitter
                // already reported the apply band; `finish` only needs it to
                // stamp the final event's clause counters, which are
                // observational. Pass 0 — the pct still terminates at exactly
                // 1.0 because the phase weights are fixed.
                progress_summary = tracker.map(|t| t.finish(0));
            }
            Err(diagnostic) => {
                let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
                verify_report = verify_report_with_failures(
                    None,
                    "graph_integrity",
                    &summary,
                    vec![diagnostic],
                );
                return compile_error(
                    model_hash,
                    compile_stats(source_count, chunk_count, &compiler),
                    verify_report,
                    &tool_version_stamp,
                );
            }
        }
    }

    CompileResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        model_hash,
        compiled_model_package_ref,
        diagnostics_ref: None,
        stats: compile_stats(source_count, chunk_count, &compiler),
        verify_report,
        progress_summary,
        budget_report,
        tool_version: Some(tool_version_stamp.clone()),
        objects: Vec::new(),
    }
}

/// The failure envelope both `compile` and `link` return once a verify report
/// has been built.
///
/// One constructor rather than an inline literal per exit, because every field
/// but the report is fixed by the fact that the operation failed: no package
/// reference, no progress summary, no budget report, and no linked objects.
fn compile_error(
    model_hash: String,
    stats: CompileStats,
    verify_report: VerifyReport,
    tool_version: &str,
) -> CompileResult {
    CompileResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        model_hash,
        compiled_model_package_ref: None,
        diagnostics_ref: None,
        stats,
        verify_report,
        progress_summary: None,
        budget_report: None,
        tool_version: Some(tool_version.to_string()),
        objects: Vec::new(),
    }
}

/// An emit (write) failure as a diagnostic.
///
/// Verifying does NOT make every later failure a write failure, so this is not
/// the whole classification: the one emit step that can still refuse the model
/// is the `.ccm` emitter, and [`ccm_emit_diagnostic`] peels that case off
/// before delegating here. What reaches this function is therefore a fault in
/// writing the output — see [`emit_failure_hint`], which classifies the
/// underlying `io::ErrorKind` rather than blaming the model.
fn emit_diagnostic(err: &anyhow::Error) -> Diagnostic {
    Diagnostic {
        code: E_COMPILE_EMIT_FAILED.to_string(),
        severity: DiagnosticSeverity::Error,
        message: err.to_string(),
        source_id: None,
        entity_path: None,
        hint: Some(emit_failure_hint(err)),
    }
}

/// The longest author-supplied identifier a diagnostic message echoes back,
/// in characters. Long enough that a snake_case facet name or declared value
/// arrives whole, short enough that the message stays a message.
const DIAGNOSTIC_ECHO_LIMIT: usize = 128;

/// One author-controlled identifier, made safe to put in a diagnostic message.
///
/// configflux-7xsy security screen. Nothing on the JSON ingest path bounds the
/// length or the charset of a facet key or a declared value, so echoing one
/// raw let a 200 KB key become a 600 KB message, and let a raw newline or the
/// ESC that opens an ANSI sequence through to whatever renders the
/// diagnostic. Every control character becomes its escaped form, and the echo
/// is cut to [`DIAGNOSTIC_ECHO_LIMIT`] characters with a trailing `…`.
/// Printable characters pass through unchanged, quotes included: the author
/// has to recognise their own identifier, and the message is prose that
/// nothing re-parses.
pub(crate) fn echo_identifier(raw: &str) -> String {
    let mut echoed = String::new();
    for character in raw.chars().take(DIAGNOSTIC_ECHO_LIMIT) {
        if character.is_control() {
            echoed.extend(character.escape_default());
        } else {
            echoed.push(character);
        }
    }
    if raw.chars().nth(DIAGNOSTIC_ECHO_LIMIT).is_some() {
        echoed.push('…');
    }
    echoed
}

/// A `.ccm` emit failure as a diagnostic, classified by what actually failed.
///
/// configflux-7xsy. The emitter parses the clauses this crate synthesizes for
/// the declared facets BEFORE it creates any directory, so a facet whose name
/// or declared value cannot be written as a condition token fails here with
/// nothing written and no filesystem involved. `E_COMPILE_EMIT_FAILED` sent
/// that author to check `--out` permissions for a fault no permission can fix.
///
/// When the error chain carries no `io::Error` AND a declared pair is genuinely
/// unexpressible, name the pair and report the model-content fault it is.
/// Requiring BOTH is what keeps a real write failure on its own code. The two
/// cannot collide today — the parse runs before the first directory is created,
/// so a model carrying an unexpressible facet never reaches an `io` call — and
/// testing the chain states that ordering here instead of depending on it
/// silently from another module.
fn ccm_emit_diagnostic(
    err: &anyhow::Error,
    domains: &crate::conditions::FacetDomains,
) -> Diagnostic {
    let no_io_fault = !err
        .chain()
        .any(|cause| cause.downcast_ref::<std::io::Error>().is_some());
    if no_io_fault {
        if let Some((facet, value)) = crate::compiler_core::unrepresentable_facet_symbol(domains) {
            return Diagnostic {
                code: E_COMPILE_INPUT_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                // Each identifier is echoed ONCE, bounded and escaped. The
                // synthesized clause is deliberately not quoted back: it
                // repeats both identifiers a second and third time, and a
                // clause rebuilt from bounded echoes would not be the text the
                // parser actually rejected.
                message: format!(
                    "Facet '{}' value '{}' cannot be expressed in the condition grammar: \
                     the clause the compiler synthesizes for the pair does not parse",
                    echo_identifier(facet),
                    echo_identifier(value)
                ),
                source_id: None,
                entity_path: None,
                hint: Some(
                    "The compiler writes each facet name and value into a condition clause, \
                     and the clause for this pair does not parse. Every symbol a model \
                     DECLARES — facet key, facet value, binding id, catalogue id, catalogue \
                     entry id — is held to the ingest charset rule (ADR-0063), which refuses \
                     this class before the emitter is reached: an id must be a snake_case \
                     identifier, and a facet value a non-empty ASCII token of letters, \
                     digits, `_`, `.` and `-`. Check this pair against that rule. If it \
                     already satisfies it, rebuild the objects this package was linked from."
                        .to_string(),
                ),
            };
        }
    }
    emit_diagnostic(err)
}

/// The failure report for a `.ccm` emit, with the check summary following the
/// code [`ccm_emit_diagnostic`] chose so that the two can never disagree. A
/// write failure keeps the summary this step has always carried.
fn ccm_emit_report(
    model_hash: Option<String>,
    err: &anyhow::Error,
    domains: &crate::conditions::FacetDomains,
) -> VerifyReport {
    let diagnostic = ccm_emit_diagnostic(err, domains);
    let summary = if diagnostic.code == E_COMPILE_EMIT_FAILED {
        "Failed to emit compiled constraint model (.ccm) artifact"
    } else {
        summary_for_diagnostic_code(&diagnostic.code)
    };
    verify_report_with_failures(model_hash, "graph_integrity", summary, vec![diagnostic])
}

/// Build the [`BudgetReport`] from the emitter's [`EmitBudgetOutcome`]
/// (configflux-9pjy.4 / ADR-0039 §5). Returns `None` — keeping the report
/// absent from the wire form — on the unbudgeted path (no budget, or a
/// budget with no `max_rss_mb`), so the default compile is byte-identical
/// to today. When the adaptive path ran, surfaces the live memo-cap shrink
/// count / final cap and the cross-partition advisory (present only when
/// the budget was approached).
fn budget_report_from_outcome(
    budget: Option<&ResourceBudget>,
    outcome: &crate::ccm_emitter::EmitBudgetOutcome,
) -> Option<BudgetReport> {
    // The report exists only when an RSS budget actually drove the path.
    // A `max_threads`-only budget does not touch the in-crate RSS lever, so
    // it produces no report (mirrors how an unbudgeted compile is silent).
    budget?.max_rss_mb?;

    let cluster_size_advisory = if outcome.cluster_size_too_large {
        let effective = outcome.effective_cluster_size.unwrap_or(usize::MAX) as u64;
        let message = if effective == u64::MAX {
            "A partition's peak RSS approached the soft budget. Consider \
             partitioning the model with --cluster-size, or raising \
             --max-rss-mb, on the next run."
                .to_string()
        } else {
            format!(
                "A partition's peak RSS approached the soft budget at the \
                 effective cluster_size of {effective}. Consider lowering \
                 --cluster-size, or raising --max-rss-mb, on the next run."
            )
        };
        Some(ClusterSizeAdvisory {
            effective_cluster_size: effective,
            message,
        })
    } else {
        None
    };

    Some(BudgetReport {
        memo_shrink_count: outcome.memo_shrink_count,
        final_memo_cap: outcome.min_final_memo_cap.map(|c| c as u64),
        cluster_size_advisory,
    })
}

/// ADR-0044 D1 (`configflux-pq2w.1`): write the deterministic, non-hashed
/// provenance sidecars for the two file-writing artifact sets emitted by a
/// compile — the CMP directory (`<out>/provenance.json`) and its sibling CCM
/// directory (`<out>/ccm/provenance.json`). Each sidecar records the tool
/// version (from `/VERSION`), the relevant schema/format versions, and the
/// SHA-256 content hashes of the primary artifacts it accompanies. The
/// sidecars are NEVER part of any hash preimage; `stamp_time` opts into a
/// wall-clock `stamped_at` (default off keeps the sidecar byte-stable).
fn write_compile_provenance(cmp_dir: &Path, ccm_dir: &Path, stamp_time: bool) -> Result<()> {
    use crate::provenance_sidecar::{hash_file, now_rfc3339_utc, ProvenanceSidecar};

    let stamped_at = if stamp_time {
        Some(now_rfc3339_utc())
    } else {
        None
    };

    // CMP sidecar: sibling of `cmp.manifest.json`.
    let cmp_manifest_name = ir::CMP_DEFAULT_MANIFEST_FILENAME;
    let mut cmp_schema_versions = BTreeMap::new();
    cmp_schema_versions.insert("cmp_manifest".to_string(), ir::CMP_MANIFEST_SCHEMA_VERSION);
    cmp_schema_versions.insert("product".to_string(), PRODUCT_SCHEMA_VERSION);
    let mut cmp_artifacts = BTreeMap::new();
    cmp_artifacts.insert(
        cmp_manifest_name.to_string(),
        hash_file(&cmp_dir.join(cmp_manifest_name))?,
    );
    ProvenanceSidecar::new(cmp_schema_versions, cmp_artifacts, stamped_at.clone())
        .write_to_dir(cmp_dir)?;

    // CCM sidecar: sibling of the top-level `ccm.manifest.json`. Record the
    // three top-level files the multi-part emitter always writes.
    let mut ccm_schema_versions = BTreeMap::new();
    ccm_schema_versions.insert(
        "ccm".to_string(),
        crate::ccm_emitter::CCM_SCHEMA_VERSION,
    );
    ccm_schema_versions.insert(
        "ccm_partition_manifest".to_string(),
        crate::ccm_emitter::CCM_PARTITION_MANIFEST_SCHEMA_VERSION,
    );
    let mut ccm_artifacts = BTreeMap::new();
    for name in ["ccm.manifest.json", "ccm.symbols.json", "partition-manifest.json"] {
        let path = ccm_dir.join(name);
        if path.exists() {
            ccm_artifacts.insert(name.to_string(), hash_file(&path)?);
        }
    }
    ProvenanceSidecar::new(ccm_schema_versions, ccm_artifacts, stamped_at)
        .write_to_dir(ccm_dir)?;

    Ok(())
}

pub fn inspect_model(request: InspectModelRequest) -> InspectionResult {
    let InspectModelRequest {
        schema_version,
        source_manifest,
        query,
    } = request;

    let source_count = source_manifest.len() as u32;
    let source_digest = hash_sources(&source_manifest);
    if schema_version != PRODUCT_SCHEMA_VERSION {
        let diagnostic = Diagnostic {
            code: E_UNSUPPORTED_SCHEMA_VERSION.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Unsupported schema_version {} (expected {})",
                schema_version, PRODUCT_SCHEMA_VERSION
            ),
            source_id: None,
            entity_path: None,
            hint: Some(format!("Set request.schema_version to {}", PRODUCT_SCHEMA_VERSION)),
        };
        return inspect_result_with_failures(
            source_digest,
            query,
            empty_inspection_summary(source_count),
            vec![diagnostic],
        );
    }

    let mut compiler = Compiler::new();
    for source in &source_manifest {
        if let Err(err) =
            compiler.add_chunk_auto(source.source_id.clone(), &source.inline_content)
        {
            return inspect_result_with_failures(
                source_digest,
                query,
                empty_inspection_summary(source_count),
                vec![map_compile_input_error(
                    &err,
                    Some(source.source_id.clone()),
                )],
            );
        }
    }

    if let Err(err) = compiler.link_and_verify() {
        return inspect_result_with_failures(
            source_digest,
            query,
            inspection_summary(compiler.get_repo(), source_count),
            vec![map_graph_error(&err)],
        );
    }

    let summary = inspection_summary(compiler.get_repo(), source_count);
    let item = match &query {
        InspectQuery::Summary => None,
        InspectQuery::Component { component_id } => {
            let Some(component) = compiler.get_repo().components.get(component_id) else {
                return inspect_result_with_failures(
                    source_digest,
                    query.clone(),
                    summary,
                    vec![inspect_unknown_component_diagnostic(component_id)],
                );
            };
            Some(inspection_item_component(component_id, component))
        }
        InspectQuery::Definition { definition_id } => {
            let Some(definition) = compiler.get_repo().definitions.get(definition_id) else {
                return inspect_result_with_failures(
                    source_digest,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_UNKNOWN_DEFINITION.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown definition '{}'", definition_id),
                        source_id: None,
                        entity_path: Some(format!("definition.{}", definition_id)),
                        hint: Some(
                            "Use `inspect summary` to list available definition IDs".to_string(),
                        ),
                    }],
                );
            };
            Some(InspectionItem::Definition {
                definition_id: definition_id.clone(),
                param_type: definition.r#type.clone(),
                inherits: definition.inherits.clone(),
                has_value: definition.value.is_some(),
                override_count: definition.overrides.len() as u32,
            })
        }
        InspectQuery::Artifact { artifact_id } => {
            let Some(artifact) = compiler.get_repo().artifacts.get(artifact_id) else {
                return inspect_result_with_failures(
                    source_digest,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_UNKNOWN_ARTIFACT.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown artifact '{}'", artifact_id),
                        source_id: None,
                        entity_path: Some(format!("artifact.{}", artifact_id)),
                        hint: Some(
                            "Use `inspect summary` to list available artifact IDs".to_string(),
                        ),
                    }],
                );
            };
            Some(InspectionItem::Artifact {
                artifact_id: artifact_id.clone(),
                name: artifact.name.clone(),
                version: artifact.version.clone(),
                hash: artifact.hash.clone(),
                source: artifact.source.clone(),
                target: artifact.target.clone(),
            })
        }
        InspectQuery::Parameter {
            component_id,
            param_key,
        } => {
            if !is_valid_snake_case_ident(component_id) || !is_valid_snake_case_ident(param_key) {
                return inspect_result_with_failures(
                    source_digest,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_QUERY_INVALID.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Invalid parameter query payload (component_id='{}', param_key='{}')",
                            component_id, param_key
                        ),
                        source_id: None,
                        entity_path: None,
                        hint: Some(
                            "Use snake_case component_id/param_key values in inspect parameter queries"
                                .to_string(),
                        ),
                    }],
                );
            }
            let Some(component) = compiler.get_repo().components.get(component_id) else {
                return inspect_result_with_failures(
                    source_digest,
                    query.clone(),
                    summary,
                    vec![inspect_unknown_component_diagnostic(component_id)],
                );
            };
            let Some(parameter) = component.params.get(param_key) else {
                return inspect_result_with_failures(
                    source_digest,
                    query.clone(),
                    summary,
                    vec![Diagnostic {
                        code: E_INSPECT_UNKNOWN_PARAMETER.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown parameter '{}.{}'", component_id, param_key),
                        source_id: None,
                        entity_path: Some(format!(
                            "component.{}.param.{}",
                            component_id, param_key
                        )),
                        hint: Some(
                            "Use `inspect component <component_id>` to list available param_keys"
                                .to_string(),
                        ),
                    }],
                );
            };
            match inspection_item_parameter(
                component_id,
                param_key,
                parameter,
                &compiler.get_repo().definitions,
            ) {
                Ok(item) => Some(item),
                Err(diagnostic) => {
                    return inspect_result_with_failures(
                        source_digest,
                        query.clone(),
                        summary,
                        vec![diagnostic],
                    );
                }
            }
        }
        InspectQuery::ScopedStats { scope } => {
            match inspection_item_scoped_stats(scope, compiler.get_repo()) {
                Ok(item) => Some(item),
                Err(diagnostic) => {
                    return inspect_result_with_failures(
                        source_digest,
                        query.clone(),
                        summary,
                        vec![diagnostic],
                    );
                }
            }
        }
    };

    inspect_result_ok(source_digest, query, summary, item)
}

fn compile_stats(source_count: u32, chunk_count: u32, compiler: &Compiler) -> CompileStats {
    let repo = compiler.get_repo();
    CompileStats {
        source_count,
        chunk_count,
        definition_count: repo.definitions.len() as u32,
        component_count: repo.components.len() as u32,
        artifact_count: repo.artifacts.len() as u32,
    }
}

fn inspection_summary(config: &crate::schema::Config, source_count: u32) -> InspectionSummary {
    let mut definition_ids: Vec<String> = config.definitions.keys().cloned().collect();
    definition_ids.sort();
    let mut component_ids: Vec<String> = config.components.keys().cloned().collect();
    component_ids.sort();
    let mut artifact_ids: Vec<String> = config.artifacts.keys().cloned().collect();
    artifact_ids.sort();

    InspectionSummary {
        source_count,
        definition_count: definition_ids.len() as u32,
        component_count: component_ids.len() as u32,
        artifact_count: artifact_ids.len() as u32,
        definition_ids,
        component_ids,
        artifact_ids,
    }
}

fn empty_inspection_summary(source_count: u32) -> InspectionSummary {
    InspectionSummary {
        source_count,
        definition_count: 0,
        component_count: 0,
        artifact_count: 0,
        definition_ids: Vec::new(),
        component_ids: Vec::new(),
        artifact_ids: Vec::new(),
    }
}

fn inspection_item_component(
    component_id: &str,
    component: &crate::schema::Component,
) -> InspectionItem {
    let mut depends_on = component.depends_on.clone();
    depends_on.sort();
    let mut param_keys: Vec<String> = component.params.keys().cloned().collect();
    param_keys.sort();
    InspectionItem::Component {
        component_id: component_id.to_string(),
        component_type: component.r#type.clone(),
        condition: component.condition.clone(),
        depends_on,
        requires: component.requires.clone(),
        param_count: param_keys.len() as u32,
        param_keys,
    }
}

fn inspect_unknown_component_diagnostic(component_id: &str) -> Diagnostic {
    Diagnostic {
        code: E_INSPECT_UNKNOWN_COMPONENT.to_string(),
        severity: DiagnosticSeverity::Error,
        message: format!("Unknown component '{}'", component_id),
        source_id: None,
        entity_path: Some(format!("component.{}", component_id)),
        hint: Some("Use `inspect summary` to list available component IDs".to_string()),
    }
}

fn inspection_item_parameter(
    component_id: &str,
    param_key: &str,
    parameter: &crate::schema::Parameter,
    definitions: &HashMap<String, crate::schema::Parameter>,
) -> std::result::Result<InspectionItem, Diagnostic> {
    let mut effective_parameter = parameter.clone();
    if let Err(err) = apply_definition_chain_to_parameter(
        &mut effective_parameter,
        definitions,
        &mut HashSet::new(),
    ) {
        return Err(Diagnostic {
            code: E_INSPECT_QUERY_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: format!(
                "Failed to materialize parameter metadata for '{}.{}': {}",
                component_id, param_key, err
            ),
            source_id: None,
            entity_path: Some(format!("component.{}.param.{}", component_id, param_key)),
            hint: Some("Verify definition inheritance for this parameter".to_string()),
        });
    }

    let mut override_conditions = Vec::new();
    collect_override_conditions(parameter, &mut override_conditions);
    override_conditions.sort();
    override_conditions.dedup();

    let mut candidate_artifact_ids = BTreeSet::new();
    if effective_parameter.r#type.as_deref() == Some("artifact") {
        collect_candidate_artifact_values(parameter, &mut candidate_artifact_ids);
    }

    Ok(InspectionItem::Parameter {
        component_id: component_id.to_string(),
        param_key: param_key.to_string(),
        inherits: parameter.inherits.clone(),
        r#type: effective_parameter.r#type.clone(),
        value: parameter.value.clone(),
        // Authored, like `value` above rather than gap-filled like the fields
        // below: `apply_definition_chain_to_parameter` never carries `facet`
        // down a definition chain, so the authored binding IS the effective one.
        facet: parameter.facet.clone(),
        unit: effective_parameter.unit.clone(),
        lifecycle: effective_parameter.lifecycle.clone(),
        safety: effective_parameter.safety.clone(),
        access: effective_parameter.access.clone(),
        req_id: effective_parameter.req_id.clone(),
        doc: effective_parameter.doc.clone(),
        override_count: count_overrides(parameter) as u32,
        override_conditions,
        candidate_artifact_ids: candidate_artifact_ids.into_iter().collect(),
    })
}

fn inspection_item_scoped_stats(
    scope: &str,
    config: &crate::schema::Config,
) -> std::result::Result<InspectionItem, Diagnostic> {
    let scope = scope.trim();
    if scope.is_empty() {
        return Err(Diagnostic {
            code: E_INSPECT_QUERY_INVALID.to_string(),
            severity: DiagnosticSeverity::Error,
            message: "Inspect scoped_stats requires a non-empty scope".to_string(),
            source_id: None,
            entity_path: Some("query.scope".to_string()),
            hint: Some(
                "Use scope values like component:<id>, platform:<id>, platform:all, or all"
                    .to_string(),
            ),
        });
    }

    let scope_roots = scope_roots_for_inspection(config, scope)?;
    let mut component_ids = BTreeSet::new();
    for root in &scope_roots {
        let closure = match dependency_closure_for_inspection(&config.components, root) {
            Ok(closure) => closure,
            Err(err) => {
                return Err(Diagnostic {
                    code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                    severity: DiagnosticSeverity::Error,
                    message: format!(
                        "Scope '{}' failed to resolve dependency closure: {}",
                        scope, err
                    ),
                    source_id: None,
                    entity_path: Some(format!("scope.{}", scope)),
                    hint: Some(
                        "Use `inspect summary` to validate scope roots and dependencies"
                            .to_string(),
                    ),
                })
            }
        };
        component_ids.extend(closure);
    }

    let component_ids: Vec<String> = component_ids.into_iter().collect();
    let component_count = component_ids.len() as u32;

    let mut parameter_count = 0_u32;
    let mut artifact_ids = BTreeSet::new();
    for component_id in &component_ids {
        let Some(component) = config.components.get(component_id) else {
            continue;
        };
        parameter_count += component.params.len() as u32;
        for parameter in component.params.values() {
            collect_parameter_artifacts(parameter, &config.definitions, &mut artifact_ids);
        }
    }

    Ok(InspectionItem::ScopedStats {
        scope: scope.to_string(),
        scope_roots,
        component_count,
        parameter_count,
        artifact_count: artifact_ids.len() as u32,
        component_ids,
        artifact_ids: artifact_ids.into_iter().collect(),
    })
}

fn scope_roots_for_inspection(
    config: &crate::schema::Config,
    scope: &str,
) -> std::result::Result<Vec<String>, Diagnostic> {
    let selectors = match crate::resolver::parse_scope_selectors(scope) {
        Ok(selectors) => selectors,
        Err(err) => {
            return Err(Diagnostic {
                code: E_INSPECT_QUERY_INVALID.to_string(),
                severity: DiagnosticSeverity::Error,
                message: format!("Invalid inspect scope '{}': {}", scope, err),
                source_id: None,
                entity_path: Some("query.scope".to_string()),
                hint: Some(
                    "Use scope values like component:<id>, platform:<id>, platform:all, or all"
                        .to_string(),
                ),
            });
        }
    };

    if selectors.len() == 1 && matches!(selectors[0], crate::resolver::ScopeSelector::All) {
        let mut roots: Vec<String> = config.components.keys().cloned().collect();
        roots.sort();
        return Ok(roots);
    }

    let mut roots = BTreeSet::new();
    for selector in selectors {
        match selector {
            crate::resolver::ScopeSelector::Component(component_id) => {
                if !config.components.contains_key(&component_id) {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown scope component '{}'", component_id),
                        source_id: None,
                        entity_path: Some(format!("scope.component.{}", component_id)),
                        hint: Some(
                            "Use `inspect summary` to list available component IDs".to_string(),
                        ),
                    });
                }
                roots.insert(component_id);
            }
            crate::resolver::ScopeSelector::Platform(component_id) => {
                let Some(component) = config.components.get(&component_id) else {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!("Unknown scope platform '{}'", component_id),
                        source_id: None,
                        entity_path: Some(format!("scope.platform.{}", component_id)),
                        hint: Some(
                            "Use `inspect summary` to list available component IDs".to_string(),
                        ),
                    });
                };
                if component.r#type.as_deref() != Some("platform") {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: format!(
                            "Scope selector 'platform:{}' does not match a platform component",
                            component_id
                        ),
                        source_id: None,
                        entity_path: Some(format!("scope.platform.{}", component_id)),
                        hint: Some(
                            "Use `component:<id>` for non-platform roots or choose a platform component"
                                .to_string(),
                        ),
                    });
                }
                roots.insert(component_id);
            }
            crate::resolver::ScopeSelector::PlatformAll => {
                for (component_id, component) in &config.components {
                    if component.r#type.as_deref() == Some("platform") {
                        roots.insert(component_id.clone());
                    }
                }
                if roots.is_empty() {
                    return Err(Diagnostic {
                        code: E_INSPECT_UNKNOWN_SCOPE.to_string(),
                        severity: DiagnosticSeverity::Error,
                        message: "Scope selector 'platform:all' matched no components".to_string(),
                        source_id: None,
                        entity_path: Some("scope.platform:all".to_string()),
                        hint: Some(
                            "Define at least one component with type = 'platform'".to_string(),
                        ),
                    });
                }
            }
            crate::resolver::ScopeSelector::All => {}
        }
    }

    Ok(roots.into_iter().collect())
}

fn dependency_closure_for_inspection(
    components: &HashMap<String, crate::schema::Component>,
    root: &str,
) -> Result<BTreeSet<String>> {
    let mut closure = BTreeSet::new();
    let mut stack = vec![root.to_string()];

    while let Some(component_id) = stack.pop() {
        if !closure.insert(component_id.clone()) {
            continue;
        }
        let component = components
            .get(&component_id)
            .with_context(|| format!("Missing component '{}'", component_id))?;
        for dependency in &component.depends_on {
            stack.push(dependency.clone());
        }
    }

    Ok(closure)
}

fn collect_parameter_artifacts(
    parameter: &crate::schema::Parameter,
    definitions: &HashMap<String, crate::schema::Parameter>,
    out: &mut BTreeSet<String>,
) {
    let mut effective_parameter = parameter.clone();
    if apply_definition_chain_to_parameter(
        &mut effective_parameter,
        definitions,
        &mut HashSet::new(),
    )
    .is_err()
    {
        return;
    }
    if effective_parameter.r#type.as_deref() != Some("artifact") {
        return;
    }
    collect_candidate_artifact_values(parameter, out);
}

fn collect_candidate_artifact_values(
    parameter: &crate::schema::Parameter,
    out: &mut BTreeSet<String>,
) {
    if let Some(crate::schema::Value::String(value)) = &parameter.value {
        if !value.is_empty() {
            out.insert(value.clone());
        }
    }
    for override_block in &parameter.overrides {
        collect_candidate_artifact_values(override_block.payload.as_ref(), out);
    }
}

fn collect_override_conditions(parameter: &crate::schema::Parameter, out: &mut Vec<String>) {
    for override_block in &parameter.overrides {
        out.push(override_block.condition.clone());
        collect_override_conditions(override_block.payload.as_ref(), out);
    }
}

fn count_overrides(parameter: &crate::schema::Parameter) -> usize {
    parameter
        .overrides
        .iter()
        .map(|override_block| 1 + count_overrides(override_block.payload.as_ref()))
        .sum()
}

fn apply_definition_chain_to_parameter(
    parameter: &mut crate::schema::Parameter,
    definitions: &HashMap<String, crate::schema::Parameter>,
    visiting: &mut HashSet<String>,
) -> Result<()> {
    let Some(definition_id) = parameter.inherits.clone() else {
        return Ok(());
    };
    apply_definition_to_parameter(parameter, &definition_id, definitions, visiting)
}

fn apply_definition_to_parameter(
    parameter: &mut crate::schema::Parameter,
    definition_id: &str,
    definitions: &HashMap<String, crate::schema::Parameter>,
    visiting: &mut HashSet<String>,
) -> Result<()> {
    if !visiting.insert(definition_id.to_string()) {
        bail!(
            "Definition inheritance cycle detected at '{}'",
            definition_id
        );
    }

    // The inspect-path twin of `link_verify::detect_definition_cycle`'s ceiling,
    // and it must stop at the same number — one ceiling per shape, or the two
    // sides disagree about which models can be inspected (configflux-l4e7).
    //
    // `visiting` holds exactly the chain walked to get here (both callers start
    // it empty), so its length after the insert above IS the current depth — the
    // quantity the link-side twin bounds with `stack.len()`. The cycle check
    // alone does not bound anything: an ACYCLIC chain inserts a new id every hop
    // and recurses once per definition. No parser limit covers this shape either,
    // because a definition chain is a FLAT map of string pointers rather than
    // nested structure (configflux-jz33), so without this the walk was bounded
    // only by the stack — measured to abort at ~30_000 definitions.
    //
    // Codeless `bail!`, matching the twin: a refusal with no code of its own
    // lands on the product mappers' generic bucket (configflux-py7w). The
    // inspect item builder wraps it into its own query diagnostic from there.
    if visiting.len() > crate::link_verify::MAX_CHAIN_DEPTH {
        bail!(
            "Definition inheritance chain exceeds the maximum supported depth {} at '{}'",
            crate::link_verify::MAX_CHAIN_DEPTH,
            definition_id
        );
    }

    let definition = definitions
        .get(definition_id)
        .with_context(|| format!("Unknown definition '{}'", definition_id))?;
    if let Some(parent_id) = definition.inherits.as_deref() {
        apply_definition_to_parameter(parameter, parent_id, definitions, visiting)?;
    }

    if parameter.r#type.is_none() {
        parameter.r#type = definition.r#type.clone();
    }
    if parameter.unit.is_none() {
        parameter.unit = definition.unit.clone();
    }
    if parameter.lifecycle.is_none() {
        parameter.lifecycle = definition.lifecycle.clone();
    }
    if parameter.safety.is_none() {
        parameter.safety = definition.safety.clone();
    }
    if parameter.access.is_none() {
        parameter.access = definition.access.clone();
    }
    if parameter.doc.is_none() {
        parameter.doc = definition.doc.clone();
    }
    // limits gap-fill removed in B-5 (ADR-0027 Decision 5): CUE resolves
    // `limits` into the emitted chunk, so re-filling it here was redundant.
    if parameter.req_id.is_none() {
        parameter.req_id = definition.req_id.clone();
    }

    visiting.remove(definition_id);
    Ok(())
}

fn is_valid_snake_case_ident(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.is_empty() || !bytes[0].is_ascii_lowercase() {
        return false;
    }

    let mut prev_underscore = false;
    for &b in bytes {
        if b.is_ascii_lowercase() || b.is_ascii_digit() {
            prev_underscore = false;
            continue;
        }
        if b == b'_' {
            if prev_underscore {
                return false;
            }
            prev_underscore = true;
            continue;
        }
        return false;
    }

    true
}

fn inspect_result_ok(
    source_digest: String,
    query: InspectQuery,
    summary: InspectionSummary,
    item: Option<InspectionItem>,
) -> InspectionResult {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    InspectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Ok,
        source_digest,
        query,
        summary,
        item,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

fn inspect_result_with_failures(
    source_digest: String,
    query: InspectQuery,
    summary: InspectionSummary,
    diagnostics: Vec<Diagnostic>,
) -> InspectionResult {
    let diagnostics = diagnostics_report(diagnostics);
    InspectionResult {
        schema_version: PRODUCT_SCHEMA_VERSION,
        status: OperationStatus::Error,
        source_digest,
        query,
        summary,
        item: None,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        diagnostics_ref: None,
        diagnostics,
    }
}

/// The complete-model checks, reported without an identity.
///
/// It does not take one and never derives one: the identity belongs to an
/// INDEX, and this function has not built one. Both callers fill the field
/// afterwards from the index they build — `verify` from
/// [`stage_in_memory`], `compile` from the index it wrote — which is what
/// keeps the two answers the same value (ADR-0056 Amendment 2 D1).
fn verify_compiler(compiler: &Compiler) -> VerifyReport {
    match compiler.link_and_verify() {
        Ok(()) => verify_report_ok(None),
        Err(err) => {
            let diagnostic = map_graph_error(&err);
            let summary = summary_for_diagnostic_code(&diagnostic.code).to_string();
            verify_report_with_failures(None, "graph_integrity", &summary, vec![diagnostic])
        }
    }
}

fn verify_report_ok(model_hash: Option<String>) -> VerifyReport {
    let diagnostics = DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics: Vec::new(),
        error_count: 0,
        warning_count: 0,
    };
    VerifyReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash,
        status: OperationStatus::Ok,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        checks: vec![VerifyCheckResult {
            check_id: "graph_integrity".to_string(),
            status: VerifyCheckStatus::Pass,
            summary: "All references and dependency constraints verified".to_string(),
            diagnostic_codes: Vec::new(),
        }],
        diagnostics_ref: None,
        diagnostics,
    }
}

fn verify_report_with_failures(
    model_hash: Option<String>,
    check_id: &str,
    summary: &str,
    diagnostics: Vec<Diagnostic>,
) -> VerifyReport {
    let diagnostics = diagnostics_report(diagnostics);
    let mut diagnostic_codes: Vec<String> = diagnostics
        .diagnostics
        .iter()
        .map(|d| d.code.clone())
        .collect();
    diagnostic_codes.sort();
    diagnostic_codes.dedup();

    VerifyReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        model_hash,
        status: OperationStatus::Error,
        error_count: diagnostics.error_count,
        warning_count: diagnostics.warning_count,
        checks: vec![VerifyCheckResult {
            check_id: check_id.to_string(),
            status: VerifyCheckStatus::Fail,
            summary: summary.to_string(),
            diagnostic_codes,
        }],
        diagnostics_ref: None,
        diagnostics,
    }
}

fn diagnostics_report(diagnostics: Vec<Diagnostic>) -> DiagnosticsReport {
    let error_count = diagnostics
        .iter()
        .filter(|diag| diag.severity == DiagnosticSeverity::Error)
        .count() as u32;
    let warning_count = diagnostics
        .iter()
        .filter(|diag| diag.severity == DiagnosticSeverity::Warning)
        .count() as u32;

    DiagnosticsReport {
        schema_version: PRODUCT_SCHEMA_VERSION,
        diagnostics,
        error_count,
        warning_count,
    }
}

fn summary_for_diagnostic_code(code: &str) -> &'static str {
    match code {
        E_UNKNOWN_COMPONENT_DEP => "Unknown dependency target",
        E_COMPONENT_DEP_CYCLE => "Component dependency cycle detected",
        E_INGEST_DUPLICATE_FACET => "Facet declared in more than one chunk",
        E_INGEST_DUPLICATE_CATALOGUE => "Catalogue declared in more than one chunk",
        E_CATALOGUE_INVALID => "Catalogue entries do not match its declared fields",
        E_BINDING_INVALID => "Binding does not resolve",
        E_OBJECT_UNIT_MISMATCH => "Object sources span more than one unit",
        E_LINK_DUPLICATE_UNIT => "Two linked objects declare the same unit",
        E_LINK_DUPLICATE_ID => "Two linked objects export the same id",
        E_LINK_UNRESOLVED_IMPORT => "No linked object declares an imported id",
        E_LINK_INTERFACE_MISMATCH => "Linked object was compiled against another interface version",
        E_LINK_OBJECT_CORRUPT => "Linked object does not match its header or its chunk names",
        E_LINK_LOCK_MISMATCH => "Linked object does not match the lockfile's pin",
        E_LINK_LOCK_UNLINKED => "Lockfile pins a unit that was not linked",
        E_LINK_LOCK_INVALID => "Lockfile could not be read as a lockfile",
        E_REQUIRES_INVALID => "Component requirement does not resolve",
        E_BINDING_NO_ACCEPTABLE_ENTRY => "No catalogue entry every requirement accepts",
        // Covers both rules this code carries (configflux-6j91): a value outside
        // a closed facet's declared domain, and a constraint naming a facet that
        // is not declared at all. The per-diagnostic message says which.
        E_FACET_VALUE_UNDECLARED => "Undeclared facet or facet value",
        E_INSPECT_UNKNOWN_COMPONENT => "Unknown component in inspect query",
        E_INSPECT_UNKNOWN_DEFINITION => "Unknown definition in inspect query",
        E_INSPECT_UNKNOWN_ARTIFACT => "Unknown artifact in inspect query",
        E_INSPECT_UNKNOWN_PARAMETER => "Unknown parameter in inspect query",
        E_INSPECT_UNKNOWN_SCOPE => "Unknown scope in inspect query",
        E_INSPECT_QUERY_INVALID => "Invalid inspect query payload",
        E_UNSUPPORTED_SCHEMA_VERSION => "Unsupported schema version",
        E_COMPILE_INPUT_INVALID => "Model ingestion failed",
        E_COMPILE_EMIT_FAILED => "Failed to emit compiled model package",
        _ => "Model verification failed",
    }
}

/// Build an accurate, non-model-blaming hint for an emit (write) failure.
///
/// By the time the compiler reaches the emit step, `link_and_verify()` has
/// already passed — but that alone does not make every later failure a write
/// failure, because the `.ccm` emitter still parses the clauses this crate
/// synthesizes from the declared facets, and a facet no condition token can
/// name fails there with nothing written (configflux-7xsy).
/// [`ccm_emit_diagnostic`] peels that case off first, so what reaches this hint
/// really is about writing the output package, which is almost always a
/// filesystem or permissions problem on the `--out` path (most commonly a
/// container bind mount that is not writable by the in-container UID). Walk the
/// `anyhow` error chain for the underlying `std::io::Error` and classify its
/// `ErrorKind` so operators are pointed at the real cause instead of being told
/// to "fix the model". `ErrorKind` is `#[non_exhaustive]`, so the wildcard arm
/// is both required and the correct home for the neutral fallback.
fn emit_failure_hint(err: &anyhow::Error) -> String {
    let io_kind = err
        .chain()
        .find_map(|cause| cause.downcast_ref::<std::io::Error>())
        .map(std::io::Error::kind);
    match io_kind {
        Some(std::io::ErrorKind::PermissionDenied) => {
            "Output directory is not writable by this process. Check the directory's \
             ownership and permissions. When running via the container toolchain, ensure the \
             bind-mounted output path is writable by the in-container user — let the cfx \
             wrapper select the user mode for your runtime (Docker Desktop / rootless / \
             native Linux), and relabel the mount with ':z' on SELinux hosts."
                .to_string()
        }
        Some(std::io::ErrorKind::ReadOnlyFilesystem) => {
            "Output path is on a read-only filesystem; choose a writable --out directory."
                .to_string()
        }
        Some(std::io::ErrorKind::NotFound) => {
            "A parent directory of the output path does not exist; create the --out parent \
             directory or point --out at an existing one."
                .to_string()
        }
        _ => {
            "Could not write the compiled model package to the output path; verify the \
             --out directory exists and is writable."
                .to_string()
        }
    }
}

/// The diagnostic a link/verify refusal reaches the caller as.
///
/// `pub(crate)` since configflux-p0jz.1: `object_compile` raises the same
/// link/verify failures over one unit, and a second mapper would let one refusal
/// carry two codes depending on which entry point saw it.
pub(crate) fn map_graph_error(err: &anyhow::Error) -> Diagnostic {
    diagnostic_for(err, None, None)
}

/// The diagnostic an INGEST refusal reaches the caller as — an `add_chunk_auto`
/// failure, which names the source it came from.
///
/// See [`map_graph_error`] for why this is crate-visible. The two mappers differ
/// in exactly two things: an ingest diagnostic carries the offending
/// `source_id`, and an ingest refusal with no code of its own has always
/// reported a remedy where a link/verify one reports none.
pub(crate) fn map_compile_input_error(
    err: &anyhow::Error,
    source_id: Option<String>,
) -> Diagnostic {
    diagnostic_for(err, source_id, Some(HINT_INGEST_GENERIC))
}

/// Read the diagnostic code off the refusal itself (configflux-py7w).
///
/// Both mappers used to recover the code by sniffing the message for one of nine
/// substrings, while nearly every validator message interpolates an authored id
/// — so an author picked the code by naming a definition after another rule's
/// phrase, and four chunks carrying ONE fault came back with four different
/// codes. The code now travels on the error as a `CodedError`, attached where
/// the rule refuses. There is deliberately no substring fallback: a prose sniff
/// anywhere keeps every arm beside it steerable, which is the defect itself.
///
/// A refusal with no code in its chain is the ordinary case rather than a
/// defect — it is how a rule says it has no dedicated code — and lands on
/// `E_COMPILE_INPUT_INVALID` with `uncoded_hint`, the generic bucket each mapper
/// already had. The message is the error's own text either way, so attaching a
/// code changed no diagnostic's wording.
fn diagnostic_for(
    err: &anyhow::Error,
    source_id: Option<String>,
    uncoded_hint: Option<&'static str>,
) -> Diagnostic {
    let coded = coded_of(err);
    let code = coded.map_or(E_COMPILE_INPUT_INVALID, |refusal| refusal.code);
    let hint = match coded {
        // A rule sharing its code with another rule carries its own remedy;
        // every other rule takes the code's.
        Some(refusal) => refusal.hint.or_else(|| hint_for(code)),
        None => uncoded_hint,
    };
    Diagnostic {
        code: code.to_string(),
        severity: DiagnosticSeverity::Error,
        message: err.to_string(),
        source_id,
        entity_path: None,
        hint: hint.map(str::to_string),
    }
}

/// The remedy for a rule that shares `E_FACET_VALUE_UNDECLARED` with the
/// closed-domain rule: a constraint asserting over a facet nothing declares
/// (configflux-6j91). Its message already carries the remedy verbatim, so the
/// hint states the RULE instead of repeating it.
pub(crate) const HINT_CONSTRAINT_FACET_UNDECLARED: &str =
    "A constraint asserts over a declared domain, never one inferred from conditions; declare \
     the facet under `facets`";

/// The remedy for a rule that shares `E_INGEST_DUPLICATE_FACET` with the
/// duplicate-facet rule: a duplicate BINDING id. A binding is a declared facet
/// (ADR-0057 §D3), so the collision is the same fault and carries the same code,
/// but the author needs to be told about the shared id space.
pub(crate) const HINT_DUPLICATE_BINDING_ID: &str =
    "A binding is a declared facet and shares the facet id space; declare each one in exactly \
     one chunk, under a name no facet uses";

/// The remedy for the authored-symbol charset rule (ADR-0063), which shares
/// `E_COMPILE_INPUT_INVALID` with every other ingest-side model fault and so
/// has to carry its own.
///
/// It states BOTH rules, because the two differ on purpose: an id is a
/// `#snakeId` (the CUE authoring constraint, mirrored in Rust for the four
/// classes the compiler interpolates into synthesized clauses), while a facet
/// VALUE is the wider file-safe token set `cfx` already enforces for an
/// environment name (ADR-0059 D1) — so `eu-west-1` stays legal where a facet
/// KEY of the same spelling would not.
pub(crate) const HINT_SYMBOL_CHARSET: &str =
    "The compiler writes every declared facet key, binding id, catalogue id, catalogue entry \
     id and facet value into a synthesized condition clause, so each has to be a symbol that \
     clause can carry. An id must be a snake_case identifier: a lowercase ASCII letter, then \
     lowercase ASCII letters, digits and single underscores — never two underscores in a row \
     — with at most one trailing underscore. A facet value must be a non-empty ASCII token of \
     letters, digits, `_`, `.` and `-`: no quote, no space, no control character. Rename the \
     symbol.";

/// The remedy for the parameter-to-facet binding rules (ADR-0064 D2).
///
/// Three of the four rules report `E_COMPILE_INPUT_INVALID`, which every other
/// ingest-side model fault also carries, and the fourth reports
/// `E_FACET_VALUE_UNDECLARED`, whose default remedy talks about a condition's
/// value rather than a declaration — so the rule set carries its own remedy
/// rather than borrowing either.
///
/// It states ALL FOUR rules, not the one that fired: an author who reaches any
/// one of them is declaring a binding for the first time, and the four together
/// are what "this parameter IS this facet's handle" means. A parameter that
/// declares no `facet` is subject to none of them.
pub(crate) const HINT_FACET_BINDING: &str =
    "A parameter that declares `facet: <name>` becomes that facet's runtime handle, so four \
     rules hold together: the facet must be declared by the model (a facet or a binding); the \
     parameter's effective type must be `string`; the parameter must author no `value` of its \
     own and no `overrides` entry may set `value` or `facet` (its value is the facet's — an \
     `overrides` entry that varies only other fields stays legal); and at most one parameter \
     in the whole model may bind a given facet. Declare the facet, fix the type, drop the \
     authored value, or remove the duplicate binding.";

/// The remedy an ingest refusal with no code of its own reports.
const HINT_INGEST_GENERIC: &str = "Fix TOML/schema issues before compile/verify";

/// The remedy a diagnostic code carries.
///
/// Keyed on the CODE, so two rules reporting one code cannot drift into two
/// remedies by accident — with the two deliberate exceptions above, which the
/// rule passes explicitly. `None` is "the message is the whole diagnostic".
fn hint_for(code: &str) -> Option<&'static str> {
    match code {
        E_UNKNOWN_COMPONENT_DEP => {
            Some("Ensure depends_on targets reference existing component IDs")
        }
        E_COMPONENT_DEP_CYCLE => Some("Break the cycle so the dependency graph is acyclic"),
        E_FACET_VALUE_UNDECLARED => Some(
            "Add the value to the facet's `values`, mark the facet `open: true`, or fix the \
             condition to use a declared value",
        ),
        E_REQUIRES_INVALID => Some(
            "A requirement must name a declared binding, and every entry it accepts must \
             be an entry of that binding's catalogue",
        ),
        E_BINDING_NO_ACCEPTABLE_ENTRY => Some(
            "Widen one of the accepts lists, or give the components separate bindings if they \
             genuinely need different entries",
        ),
        E_INGEST_DUPLICATE_CATALOGUE => Some(
            "A catalogue is a pack-global table; declare it in exactly one chunk and bind to it \
             from the others",
        ),
        E_CATALOGUE_INVALID => Some(
            "Every entry must supply exactly the declared fields, each with a finite value of \
             that field's declared type",
        ),
        E_INGEST_DUPLICATE_FACET => {
            Some("A facet is a pack-global domain; declare each facet in exactly one chunk")
        }
        E_BINDING_INVALID => Some(
            "A binding must name a declared catalogue, default to one of its \
             entries, and derive from a declared facet or binding",
        ),
        _ => None,
    }
}

/// The source-manifest digest (ADR-0056 §9.1): the SHA-256 of the per-source
/// content digests, sorted ascending as unsigned byte sequences and
/// concatenated raw.
///
/// `source_id` is deliberately absent from both the preimage and the sort key.
/// It is a `--source` argument exactly as the caller spelled it, never
/// canonicalized, so admitting it would make this value depend on where a tree
/// is checked out and on whether the path was written relative or absolute —
/// the invariant ADR-0056 §5 states as *no path string may enter any hash
/// preimage*.
///
/// Every element is exactly 32 bytes wide, so the concatenation is unambiguous
/// and carries no separator, prefix or terminator. Equal contents are fed twice
/// rather than de-duplicated: this runs before ingest, which is the stage that
/// rejects a duplicate `chunk_hash` (§3), so the function stays total over
/// manifests ingest will go on to reject. Sort stability is irrelevant — equal
/// keys carry equal payloads, so tie order cannot move the preimage.
fn hash_sources(sources: &[SourceManifestEntry]) -> String {
    let mut content_digests: Vec<[u8; 32]> = Vec::with_capacity(sources.len());
    for source in sources {
        let mut content_digest = [0u8; 32];
        content_digest.copy_from_slice(&Sha256::digest(source.inline_content.as_bytes()));
        content_digests.push(content_digest);
    }
    content_digests.sort_unstable();

    let mut hasher = Sha256::new();
    for content_digest in &content_digests {
        hasher.update(content_digest);
    }

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coded_error::{coded, coded_with_hint};
    use crate::loader_api::{open_model, OpenModelRequest};
    use crate::scenario_test_support::unique_temp_path;
    use std::path::{Path, PathBuf};

    fn verify_with_chunk(content: &str) -> VerifyReport {
        verify_model(VerifyModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![SourceManifestEntry {
                source_id: "test.toml".to_string(),
                inline_content: content.to_string(),
            }],
        })
    }

    fn inspect_with_chunks(query: InspectQuery, chunks: &[(&str, &str)]) -> InspectionResult {
        inspect_model(InspectModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: chunks
                .iter()
                .map(|(source_id, inline_content)| SourceManifestEntry {
                    source_id: (*source_id).to_string(),
                    inline_content: (*inline_content).to_string(),
                })
                .collect(),
            query,
        })
    }

    #[test]
    fn verify_unknown_dependency_emits_expected_code() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.motor]
            type = "actuator"
            depends_on = ["missing_component"]
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Error);
        assert_eq!(
            report.checks[0].diagnostic_codes,
            vec![E_UNKNOWN_COMPONENT_DEP.to_string()]
        );
    }

    #[test]
    fn product_schema_version_is_five() {
        // ADR-0047 §2 advanced the product-contract discriminator 2 -> 3 (the
        // facet namespace + model_hash rotation). ADR-0054 §7 advanced it
        // 3 -> 4: the `constraints` namespace, and with it `condition` reverting
        // to inclusion-selector-only semantics. ADR-0057 §D7 advances it
        // 4 -> 5: a resolved snapshot delivers each requirement's catalogue
        // entry inside the requiring component. A v4 request is rejected with
        // E_UNSUPPORTED_SCHEMA_VERSION; there are no back-compat shims.
        assert_eq!(PRODUCT_SCHEMA_VERSION, 5);
    }

    #[test]
    fn map_graph_error_reads_the_code_and_remedy_off_the_refusal() {
        let diag = map_graph_error(&coded(
            E_FACET_VALUE_UNDECLARED,
            "Condition value 'mars' is not in the closed facet 'region' domain [eu, us]"
                .to_string(),
        ));
        assert_eq!(diag.code, E_FACET_VALUE_UNDECLARED);
        assert_eq!(diag.hint, hint_for(E_FACET_VALUE_UNDECLARED).map(str::to_string));
        assert_eq!(
            diag.message,
            "Condition value 'mars' is not in the closed facet 'region' domain [eu, us]",
            "attaching a code must not change what the author reads"
        );
    }

    #[test]
    fn map_graph_error_prefers_the_rules_own_remedy_over_the_codes() {
        // configflux-6j91 shares E_FACET_VALUE_UNDECLARED with the closed-domain
        // rule above and carries its own remedy. The end-to-end guards are
        // //compiler:constraint_facet_diagnostic_test and
        // //compiler:diagnostic_code_routing_test, which drive the real compile
        // path rather than hand-building the refusal.
        let diag = map_graph_error(&coded_with_hint(
            E_FACET_VALUE_UNDECLARED,
            HINT_CONSTRAINT_FACET_UNDECLARED,
            "Constraint 'pinned_arch' references facet 'arch', which is not declared under \
             `facets`: declare the facet with its value domain, or remove it from the constraint"
                .to_string(),
        ));
        assert_eq!(diag.code, E_FACET_VALUE_UNDECLARED);
        assert_eq!(diag.hint.as_deref(), Some(HINT_CONSTRAINT_FACET_UNDECLARED));
        assert_ne!(
            diag.hint.as_deref(),
            hint_for(E_FACET_VALUE_UNDECLARED),
            "the two rules sharing this code must not collapse onto one remedy"
        );
    }

    #[test]
    fn neither_mapper_can_be_steered_by_message_text() {
        // configflux-py7w: every phrase either mapper ever routed on, in ONE
        // uncoded message. A surviving substring arm — in either mapper, for any
        // one of them — turns this red, which is the property the fix rests on:
        // an authored id lands inside a message, so a prose sniff anywhere is
        // steerable by the author.
        let message = "Catalogue 'x' Binding 'y' depends_on unknown component \
                       dependency cycle detected closed facet is not declared under `facets` \
                       requires slot ' has no entry every requirement accepts \
                       declared in more than one chunk";
        let refusal = anyhow::Error::msg(message);

        let graph = map_graph_error(&refusal);
        assert_eq!(graph.code, E_COMPILE_INPUT_INVALID);
        assert_eq!(graph.hint, None, "a link/verify refusal with no code has no remedy");
        assert_eq!(graph.message, message);

        let ingest = map_compile_input_error(&refusal, Some("00_chunk.toml".to_string()));
        assert_eq!(ingest.code, E_COMPILE_INPUT_INVALID);
        assert_eq!(ingest.hint.as_deref(), Some(HINT_INGEST_GENERIC));
        assert_eq!(ingest.source_id.as_deref(), Some("00_chunk.toml"));
    }

    #[test]
    fn map_compile_input_error_reads_the_duplicate_facet_code_and_keeps_the_source() {
        let diag = map_compile_input_error(
            &coded(
                E_INGEST_DUPLICATE_FACET,
                "Facet 'region' is declared in more than one chunk".to_string(),
            ),
            Some("01_chunk.toml".to_string()),
        );
        assert_eq!(diag.code, E_INGEST_DUPLICATE_FACET);
        assert_eq!(diag.hint, hint_for(E_INGEST_DUPLICATE_FACET).map(str::to_string));
        assert_eq!(diag.source_id.as_deref(), Some("01_chunk.toml"));
    }

    #[test]
    fn verify_closed_facet_undeclared_value_emits_expected_code() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "us"]
            default = "eu"

            [components.motor]
            type = "actuator"
            condition = "region == 'mars'"
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Error);
        assert_eq!(
            report.checks[0].diagnostic_codes,
            vec![E_FACET_VALUE_UNDECLARED.to_string()]
        );
    }

    #[test]
    fn verify_closed_facet_declared_value_passes() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [facets.region]
            values = ["eu", "us"]
            default = "eu"

            [components.motor]
            type = "actuator"
            condition = "region == 'us'"
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Ok);
    }

    #[test]
    fn verify_cycle_emits_expected_code() {
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.alpha]
            type = "actuator"
            depends_on = ["beta"]

            [components.beta]
            type = "actuator"
            depends_on = ["alpha"]
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Error);
        assert_eq!(
            report.checks[0].diagnostic_codes,
            vec![E_COMPONENT_DEP_CYCLE.to_string()]
        );
    }

    #[test]
    fn verify_diamond_is_accepted() {
        // ADR-0048: a diamond (shared `shared` reached from `root` via both
        // `left` and `right`) is a permitted DAG, not an error.
        let chunk = r#"
            package = "p1"
            version = "1.0"

            [components.root]
            type = "actuator"
            depends_on = ["left", "right"]

            [components.left]
            type = "actuator"
            depends_on = ["shared"]

            [components.right]
            type = "actuator"
            depends_on = ["shared"]

            [components.shared]
            type = "actuator"
        "#;

        let report = verify_with_chunk(chunk);
        assert_eq!(report.status, OperationStatus::Ok);
        assert_eq!(report.error_count, 0);
    }

    #[test]
    fn inspect_summary_includes_component_definition_artifact_ids() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Summary,
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        assert_eq!(result.summary.source_count, 2);
        assert!(result
            .summary
            .component_ids
            .contains(&"thermal_control".to_string()));
        assert!(result
            .summary
            .definition_ids
            .contains(&"safe_flow".to_string()));
        assert!(result
            .summary
            .artifact_ids
            .contains(&"hydra_x200_single_driver".to_string()));
    }

    #[test]
    fn inspect_component_returns_component_item() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Component {
                component_id: "thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        match result.item {
            Some(InspectionItem::Component {
                component_id,
                param_count,
                ..
            }) => {
                assert_eq!(component_id, "thermal_control");
                assert!(param_count > 0);
            }
            other => panic!("expected component item, got {other:?}"),
        }
    }

    #[test]
    fn inspect_unknown_component_emits_expected_code() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Component {
                component_id: "missing_component".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Error);
        assert_eq!(result.error_count, 1);
        assert_eq!(result.diagnostics.diagnostics.len(), 1);
        assert_eq!(
            result.diagnostics.diagnostics[0].code,
            E_INSPECT_UNKNOWN_COMPONENT.to_string()
        );
    }

    #[test]
    fn inspect_parameter_returns_metadata_and_artifact_candidates() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Parameter {
                component_id: "thermal_control".to_string(),
                param_key: "control_driver".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        match result.item {
            Some(InspectionItem::Parameter {
                component_id,
                param_key,
                r#type,
                value,
                override_count,
                candidate_artifact_ids,
                ..
            }) => {
                assert_eq!(component_id, "thermal_control");
                assert_eq!(param_key, "control_driver");
                assert_eq!(r#type.as_deref(), Some("artifact"));
                assert_eq!(
                    value,
                    Some(crate::schema::Value::String(
                        "hydra_x200_single_driver".to_string()
                    ))
                );
                assert_eq!(override_count, 2);
                assert_eq!(
                    candidate_artifact_ids,
                    vec![
                        "aeroflux_a9_driver".to_string(),
                        "hydra_x200_dual_driver".to_string(),
                        "hydra_x200_single_driver".to_string()
                    ]
                );
            }
            other => panic!("expected parameter item, got {other:?}"),
        }
    }

    #[test]
    fn inspect_scoped_stats_counts_dependency_closure() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Ok);
        match result.item {
            Some(InspectionItem::ScopedStats {
                component_count,
                parameter_count,
                artifact_count,
                component_ids,
                artifact_ids,
                ..
            }) => {
                assert_eq!(component_count, 2);
                assert_eq!(parameter_count, 3);
                assert_eq!(artifact_count, 3);
                assert_eq!(
                    component_ids,
                    vec!["power_bus".to_string(), "thermal_control".to_string()]
                );
                assert_eq!(
                    artifact_ids,
                    vec![
                        "aeroflux_a9_driver".to_string(),
                        "hydra_x200_dual_driver".to_string(),
                        "hydra_x200_single_driver".to_string()
                    ]
                );
            }
            other => panic!("expected scoped_stats item, got {other:?}"),
        }
    }

    #[test]
    fn inspect_unknown_parameter_emits_expected_code() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let result = inspect_with_chunks(
            InspectQuery::Parameter {
                component_id: "thermal_control".to_string(),
                param_key: "missing_param".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(result.status, OperationStatus::Error);
        assert_eq!(result.error_count, 1);
        assert_eq!(
            result.diagnostics.diagnostics[0].code,
            E_INSPECT_UNKNOWN_PARAMETER.to_string()
        );
    }

    #[test]
    fn inspect_unknown_scope_and_invalid_payload_emit_expected_codes() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let unknown_scope = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:missing_component".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );
        assert_eq!(unknown_scope.status, OperationStatus::Error);
        assert_eq!(
            unknown_scope.diagnostics.diagnostics[0].code,
            E_INSPECT_UNKNOWN_SCOPE.to_string()
        );

        let invalid_payload = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:Bad_Name".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );
        assert_eq!(invalid_payload.status, OperationStatus::Error);
        assert_eq!(
            invalid_payload.diagnostics.diagnostics[0].code,
            E_INSPECT_QUERY_INVALID.to_string()
        );
    }

    #[test]
    fn inspect_scoped_stats_payload_is_deterministic() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let first = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );
        let second = inspect_with_chunks(
            InspectQuery::ScopedStats {
                scope: "component:thermal_control".to_string(),
            },
            &[
                ("scenarios/s1/00_definitions.toml", defs),
                ("scenarios/s1/10_components.toml", comps),
            ],
        );

        assert_eq!(first.status, OperationStatus::Ok);
        assert_eq!(second.status, OperationStatus::Ok);
        let first_json = serde_json::to_string(&first).expect("serialize first");
        let second_json = serde_json::to_string(&second).expect("serialize second");
        assert_eq!(first_json, second_json);
    }

    #[test]
    fn compile_emits_manifest_ref_for_loader_handoff() {
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let output_dir = unique_temp_path("cfx-compile", "handoff");

        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "scenarios/s1/00_definitions.toml".to_string(),
                    inline_content: defs.to_string(),
                },
                SourceManifestEntry {
                    source_id: "scenarios/s1/10_components.toml".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(output_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });

        assert_eq!(result.status, OperationStatus::Ok);
        let cmp_manifest_ref = result.compiled_model_package_ref.expect("cmp manifest ref");
        assert!(
            cmp_manifest_ref.ends_with(ir::CMP_DEFAULT_MANIFEST_FILENAME),
            "cmp_manifest_ref={cmp_manifest_ref}"
        );
        assert!(Path::new(&cmp_manifest_ref).exists());

        let open_result = open_model(OpenModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            cmp_manifest_ref: cmp_manifest_ref.clone(),
        });
        assert_eq!(open_result.status, OperationStatus::Ok);
        assert_eq!(open_result.model_hash, Some(result.model_hash));

        std::fs::remove_dir_all(output_dir).ok();
    }

    // Regression for the dogfood-trial failure: a write/permission failure on
    // the output path (e.g. a container bind mount not writable by the
    // in-container UID) must be reported as a FILESYSTEM cause, not as a model
    // error. The companion `verify_*_emits_expected_code` tests above prove the
    // genuine-model-error path still produces its model-blaming diagnostics, so
    // the two together pin both halves of the classification.
    #[cfg(unix)]
    #[test]
    fn compile_emit_permission_denied_reports_filesystem_cause() {
        use std::os::unix::fs::PermissionsExt;

        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");

        let base = unique_temp_path("configflux-emit", "perm");
        let ro_parent = base.join("ro");
        std::fs::create_dir_all(&ro_parent).expect("create ro parent");
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o555))
            .expect("chmod 0555");

        // Root bypasses mode bits — if a write into the read-only dir still
        // succeeds we cannot trigger PermissionDenied; skip rather than fail.
        if std::fs::File::create(ro_parent.join(".root_probe")).is_ok() {
            std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755)).ok();
            std::fs::remove_dir_all(&base).ok();
            eprintln!("skipping: running as root, mode bits do not deny writes");
            return;
        }

        // The output lands UNDER the read-only parent, so the emit write fails.
        let output_dir = ro_parent.join("cmp");
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "scenarios/s1/00_definitions.toml".to_string(),
                    inline_content: defs.to_string(),
                },
                SourceManifestEntry {
                    source_id: "scenarios/s1/10_components.toml".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(output_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });

        // Restore perms before asserting, so cleanup always succeeds.
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755)).ok();

        assert_eq!(result.status, OperationStatus::Error);
        let diags = &result.verify_report.diagnostics.diagnostics;
        assert_eq!(diags.len(), 1, "expected one emit diagnostic, got {diags:?}");
        let diag = &diags[0];
        // The frozen diagnostic code is preserved (contract stability).
        assert_eq!(diag.code, E_COMPILE_EMIT_FAILED);
        let hint = diag.hint.clone().expect("emit diagnostic carries a hint");
        assert!(
            hint.contains("writable"),
            "hint should name the filesystem/permission cause, got: {hint}"
        );
        assert!(
            !hint.contains("Fix model"),
            "hint must not blame the model, got: {hint}"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    // ---- configflux-7xsy: a facet the condition grammar cannot express ----

    /// The shipped hero example with `00_definitions.json` rewritten by one
    /// `needle` -> `patch` substitution, compiled into a fresh temp directory.
    /// The example is the model the documentation teaches, so a fault injected
    /// into it is a fault an author can actually reach.
    fn compile_hero_example(label: &str, needle: &str, patch: &str) -> (CompileResult, PathBuf) {
        let defs = include_str!("../../examples/00-service-multi-env/00_definitions.json");
        let comps = include_str!("../../examples/00-service-multi-env/10_components.json");
        assert_eq!(
            defs.matches(needle).count(),
            1,
            "needle {needle:?} must identify exactly one field to edit"
        );
        let output_dir = unique_temp_path("configflux-7xsy", label);
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "examples/00-service-multi-env/00_definitions.json".to_string(),
                    inline_content: defs.replace(needle, patch),
                },
                SourceManifestEntry {
                    source_id: "examples/00-service-multi-env/10_components.json".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(output_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });
        (result, output_dir)
    }

    /// A facet KEY carrying a space that NOTHING references survives ingest
    /// (ADR-0027 Decision 4 removed the Rust mirror of the CUE snake_case
    /// rule) and survives verification, then breaks the symbol-introduction
    /// clause the emitter synthesizes for it. The fault is in the model, so it
    /// must not wear the code that means "the write failed".
    #[test]
    fn compile_unexpressible_facet_key_is_a_model_fault_not_an_emit_failure() {
        // `replica_class` is named by no constraint and no component
        // condition, so renaming it reaches the emitter rather than the
        // earlier constraint parse — which is the case that was already
        // accurate and must not move.
        let (result, output_dir) =
            compile_hero_example("key", "\"replica_class\": {", "\"replica class\": {");
        assert_eq!(result.status, OperationStatus::Error);
        let diags = &result.verify_report.diagnostics.diagnostics;
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        let diag = &diags[0];
        assert_eq!(
            diag.code, E_COMPILE_INPUT_INVALID,
            "a model-content fault must not wear the emit code: {diag:?}"
        );
        assert!(
            diag.message.contains("replica class"),
            "message must name the offending facet, got: {}",
            diag.message
        );
        let hint = diag.hint.clone().expect("diagnostic carries a hint");
        assert!(
            hint.contains("snake_case"),
            "hint must point at the identifier convention, got: {hint}"
        );
        assert!(
            !hint.contains("writable"),
            "hint must not send the author to the filesystem, got: {hint}"
        );
        std::fs::remove_dir_all(&output_dir).ok();
    }

    /// The VALUE half of the same rule, as ADR-0063 D2 leaves it.
    ///
    /// A value carrying ONE quote character used to be legal — the emitter
    /// falls back to double quotes for exactly that case, so the clause parsed
    /// and the model compiled. D2 supersedes that: a facet value is now a
    /// file-safe token, so either quote is refused at INGEST and the emitter is
    /// never reached. What the refusal owes the author is unchanged, which is
    /// what the two cases assert — it names the owning facet and echoes the
    /// value, so the author can find the field they wrote.
    ///
    /// The both-quote case is kept because it is configflux-mrm6 case 3: the
    /// exact class the emitter's old hint text told the author was refused,
    /// while the compiler accepted it and silently truncated the value. The
    /// control is a value the token rule keeps legal, which is what stops this
    /// test from passing for a compiler that refuses everything.
    #[test]
    fn compile_unexpressible_facet_value_names_the_value() {
        let (ok, ok_dir) = compile_hero_example(
            "value-ok",
            "\"values\": [\"off\", \"on\"]",
            "\"values\": [\"off\", \"on\", \"on-call.2\"]",
        );
        assert_eq!(
            ok.status,
            OperationStatus::Ok,
            "a token value stays legal: {:?}",
            ok.verify_report.diagnostics.diagnostics
        );
        std::fs::remove_dir_all(&ok_dir).ok();

        for (label, patch, wanted) in [
            ("value-one-quote", "\"values\": [\"off\", \"on\", \"o'n\"]", "o'n"),
            (
                "value-both-quotes",
                "\"values\": [\"off\", \"on\", \"o'n\\\"x\"]",
                "o'n\"x",
            ),
        ] {
            let (result, output_dir) =
                compile_hero_example(label, "\"values\": [\"off\", \"on\"]", patch);
            assert_eq!(result.status, OperationStatus::Error, "{label}");
            let diags = &result.verify_report.diagnostics.diagnostics;
            assert_eq!(diags.len(), 1, "{label}: expected one diagnostic, got {diags:?}");
            let diag = &diags[0];
            assert_eq!(
                diag.code, E_COMPILE_INPUT_INVALID,
                "{label}: a model-content fault must not wear the emit code: {diag:?}"
            );
            assert!(
                diag.message.contains("beta_dashboard") && diag.message.contains(wanted),
                "{label}: message must name the facet and the value, got: {}",
                diag.message
            );
            std::fs::remove_dir_all(&output_dir).ok();
        }
    }

    /// configflux-7xsy security screen. A facet key is author-controlled and
    /// unbounded — nothing on the JSON ingest path limits its length — so the
    /// echo must not grow with it. Ten kilobytes of key still has to produce a
    /// message an author can read, carrying enough of the key to recognise it
    /// and no more.
    #[test]
    fn compile_unexpressible_facet_key_echo_is_bounded() {
        let huge = format!("replica class {}", "x".repeat(10_000));
        let (result, output_dir) =
            compile_hero_example("huge-key", "\"replica_class\": {", &format!("\"{huge}\": {{"));
        assert_eq!(result.status, OperationStatus::Error);
        let diags = &result.verify_report.diagnostics.diagnostics;
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        let diag = &diags[0];
        assert_eq!(diag.code, E_COMPILE_INPUT_INVALID);
        assert!(
            !diag.message.contains(&huge),
            "the whole key must not reach the message ({} bytes)",
            diag.message.len()
        );
        assert!(
            diag.message.len() < 512,
            "message must stay bounded, got {} bytes",
            diag.message.len()
        );
        assert!(
            diag.message.contains("replica class xxx") && diag.message.contains('…'),
            "message must carry an elided prefix of the key, got: {}",
            diag.message
        );
        std::fs::remove_dir_all(&output_dir).ok();
    }

    /// The same echo must not carry a control character into whatever renders
    /// the diagnostic: a raw newline forges a line of output, and the ESC that
    /// opens an ANSI sequence acts on the terminal it reaches. Both are
    /// escaped, and the author still sees which key was refused.
    #[test]
    fn compile_unexpressible_facet_key_echo_escapes_control_characters() {
        let (result, output_dir) = compile_hero_example(
            "ctrl-key",
            "\"replica_class\": {",
            "\"repl\\nica\\u001b[31m class\": {",
        );
        assert_eq!(result.status, OperationStatus::Error);
        let diags = &result.verify_report.diagnostics.diagnostics;
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        let diag = &diags[0];
        assert_eq!(diag.code, E_COMPILE_INPUT_INVALID);
        assert!(
            !diag.message.contains('\n') && !diag.message.contains('\u{1b}'),
            "no raw control character may reach the message, got: {:?}",
            diag.message
        );
        assert!(
            diag.message.contains("repl\\nica\\u{1b}[31m class"),
            "the key must be echoed in escaped form, got: {:?}",
            diag.message
        );
        std::fs::remove_dir_all(&output_dir).ok();
    }

    /// The io half must be untouched. A model whose facets are all expressible
    /// — the hero example verbatim — writing into an unwritable path keeps the
    /// emit code and the filesystem hint, so the model-content classification
    /// cannot swallow a genuine write failure.
    #[cfg(unix)]
    #[test]
    fn compile_emit_failure_with_declared_facets_still_blames_the_filesystem() {
        use std::os::unix::fs::PermissionsExt;

        let base = unique_temp_path("configflux-7xsy", "ro");
        let ro_parent = base.join("ro");
        std::fs::create_dir_all(&ro_parent).expect("create ro parent");
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o555))
            .expect("chmod 0555");
        // Root bypasses mode bits — if a write into the read-only dir still
        // succeeds we cannot trigger the failure; skip rather than fail.
        if std::fs::File::create(ro_parent.join(".root_probe")).is_ok() {
            std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755)).ok();
            std::fs::remove_dir_all(&base).ok();
            eprintln!("skipping: running as root, mode bits do not deny writes");
            return;
        }

        let defs = include_str!("../../examples/00-service-multi-env/00_definitions.json");
        let comps = include_str!("../../examples/00-service-multi-env/10_components.json");
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "examples/00-service-multi-env/00_definitions.json".to_string(),
                    inline_content: defs.to_string(),
                },
                SourceManifestEntry {
                    source_id: "examples/00-service-multi-env/10_components.json".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(ro_parent.join("cmp").to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });
        // Restore perms before asserting, so cleanup always succeeds.
        std::fs::set_permissions(&ro_parent, std::fs::Permissions::from_mode(0o755)).ok();

        assert_eq!(result.status, OperationStatus::Error);
        let diags = &result.verify_report.diagnostics.diagnostics;
        assert_eq!(diags.len(), 1, "expected one diagnostic, got {diags:?}");
        assert_eq!(diags[0].code, E_COMPILE_EMIT_FAILED);
        let hint = diags[0].hint.clone().expect("emit diagnostic carries a hint");
        assert!(
            hint.contains("writable"),
            "hint must name the filesystem cause, got: {hint}"
        );
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn budget_report_absent_for_unbudgeted_compile() {
        // configflux-9pjy.4 / ADR-0039 §5: the default (unbudgeted) compile
        // must carry NO budget_report — the field is absent from the wire
        // form, keeping the default result byte-identical to today.
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
        let output_dir = unique_temp_path("configflux-9pjy4", "unbudgeted");
        let result = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: vec![
                SourceManifestEntry {
                    source_id: "scenarios/s1/00_definitions.toml".to_string(),
                    inline_content: defs.to_string(),
                },
                SourceManifestEntry {
                    source_id: "scenarios/s1/10_components.toml".to_string(),
                    inline_content: comps.to_string(),
                },
            ],
            output_dir: Some(output_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: None,
            stamp_time: false,
        });
        assert_eq!(result.status, OperationStatus::Ok);
        assert!(
            result.budget_report.is_none(),
            "unbudgeted compile must not carry a budget_report"
        );
        // And it must not serialize a `budget_report` key at all.
        let json = serde_json::to_string(&result).expect("serialize result");
        assert!(
            !json.contains("budget_report"),
            "budget_report key must be absent from the unbudgeted wire form"
        );
        std::fs::remove_dir_all(&output_dir).ok();
    }

    #[test]
    fn budget_report_from_outcome_shapes_advisory_correctly() {
        // configflux-9pjy.4 / ADR-0039 §5 (AC2): the report exists only for
        // an RSS-bearing budget, and the cluster-size advisory appears ONLY
        // when the emitter flagged `cluster_size_too_large`.
        use crate::ccm_emitter::EmitBudgetOutcome;

        let rss_budget = ResourceBudget {
            max_rss_mb: Some(512),
            max_threads: None,
        };

        // No advisory flagged ⇒ report present, advisory absent.
        let calm = EmitBudgetOutcome {
            memo_shrink_count: 2,
            min_final_memo_cap: Some(8192),
            cluster_size_too_large: false,
            effective_cluster_size: Some(usize::MAX),
        };
        let report = budget_report_from_outcome(Some(&rss_budget), &calm)
            .expect("an RSS budget yields a report");
        assert_eq!(report.memo_shrink_count, 2);
        assert_eq!(report.final_memo_cap, Some(8192));
        assert!(
            report.cluster_size_advisory.is_none(),
            "no advisory when cluster_size_too_large is false"
        );

        // Advisory flagged ⇒ advisory present and names the effective size.
        let tight = EmitBudgetOutcome {
            memo_shrink_count: 5,
            min_final_memo_cap: Some(4096),
            cluster_size_too_large: true,
            effective_cluster_size: Some(2048),
        };
        let report = budget_report_from_outcome(Some(&rss_budget), &tight)
            .expect("an RSS budget yields a report");
        let advisory = report
            .cluster_size_advisory
            .expect("advisory present when flagged");
        assert_eq!(advisory.effective_cluster_size, 2048);
        assert!(advisory.message.contains("2048"));

        // A threads-only budget (no max_rss_mb) ⇒ no report at all.
        let threads_only = ResourceBudget {
            max_rss_mb: None,
            max_threads: Some(4),
        };
        assert!(
            budget_report_from_outcome(Some(&threads_only), &tight).is_none(),
            "a threads-only budget must not produce an RSS adaptation report"
        );
        // No budget ⇒ no report.
        assert!(budget_report_from_outcome(None, &tight).is_none());
    }

    #[test]
    fn budget_derived_cluster_size_matches_explicit_cluster_size_bytes() {
        // configflux-9pjy.4 / ADR-0039 §5/§8 (AC2): a budget that derives a
        // cluster_size must produce the SAME partition-layout bytes as the
        // operator passing that effective cluster_size explicitly with no
        // budget. The budget additionally sets a byte-neutral memo cap, so
        // the only thing that could differ is the cache size — which never
        // touches output bytes. This pins "the advisory/budget metadata
        // path does not perturb the byte-stable partition layout."
        let defs = include_str!("../scenarios/s1_water_pump/smoke/cue/00_definitions.json");
        let comps = include_str!("../scenarios/s1_water_pump/smoke/cue/10_components.json");
        let manifest = vec![
            SourceManifestEntry {
                source_id: "scenarios/s1/00_definitions.toml".to_string(),
                inline_content: defs.to_string(),
            },
            SourceManifestEntry {
                source_id: "scenarios/s1/10_components.toml".to_string(),
                inline_content: comps.to_string(),
            },
        ];
        let base = unique_temp_path("configflux-9pjy4", "clustermatch");

        // A tiny budget: below the 64 MiB overhead floor, so derive_knobs
        // partitions every nontrivial model (effective cluster_size = 1).
        let budget_dir = base.join("budgeted");
        let budgeted = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: manifest.clone(),
            output_dir: Some(budget_dir.to_string_lossy().into_owned()),
            cluster_size: None,
            budget: Some(ResourceBudget {
                max_rss_mb: Some(1),
                max_threads: None,
            }),
            stamp_time: false,
        });
        assert_eq!(budgeted.status, OperationStatus::Ok, "budgeted compile ok");
        let report = budgeted
            .budget_report
            .as_ref()
            .expect("an RSS budget yields a report");
        let effective = report
            .cluster_size_advisory
            .as_ref()
            .map(|a| a.effective_cluster_size)
            .unwrap_or(1);

        // The explicit-equivalent: same effective cluster_size, NO budget.
        let explicit_dir = base.join("explicit");
        let explicit = compile_model(CompileModelRequest {
            schema_version: PRODUCT_SCHEMA_VERSION,
            source_manifest: manifest,
            output_dir: Some(explicit_dir.to_string_lossy().into_owned()),
            cluster_size: Some(effective.max(1) as usize),
            budget: None,
            stamp_time: false,
        });
        assert_eq!(explicit.status, OperationStatus::Ok, "explicit compile ok");
        assert!(
            explicit.budget_report.is_none(),
            "explicit (unbudgeted) compile carries no report"
        );

        // The two .ccm trees must be byte-identical, file for file. Compare
        // the recursively-collected (relative path -> bytes) maps.
        let budget_ccm = budget_dir.join("ccm");
        let explicit_ccm = explicit_dir.join("ccm");
        let budget_files = collect_dir_bytes(&budget_ccm);
        let explicit_files = collect_dir_bytes(&explicit_ccm);
        assert_eq!(
            budget_files, explicit_files,
            "budget-derived cluster_size must yield byte-identical partition layout \
             to the same explicit cluster_size"
        );

        std::fs::remove_dir_all(&base).ok();
    }

    /// Recursively collect a directory's files into a sorted
    /// `relative-path -> bytes` map for byte-for-byte tree comparison.
    fn collect_dir_bytes(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        fn walk(
            dir: &Path,
            prefix: &Path,
            out: &mut std::collections::BTreeMap<String, Vec<u8>>,
        ) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let rel = prefix.join(entry.file_name());
                if path.is_dir() {
                    walk(&path, &rel, out);
                } else if let Ok(bytes) = std::fs::read(&path) {
                    out.insert(rel.to_string_lossy().into_owned(), bytes);
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        walk(root, Path::new(""), &mut out);
        out
    }
}
