// SPDX-License-Identifier: BUSL-1.1

// configflux-y2ai: the resolve-hash pre-image and the selection-state pre-image
// live in the leaf `crate::resolve_hash` module, which `runtime_api` imports
// too, so the loader's emission and the runtime's recomputation cannot drift
// apart. `SelectionStateCanonical` is also the whole pre-image of
// `compute_selection_state_hash` below. `ResolveHashCanonical` itself is
// reached only by the unit test that holds `RESOLVE_HASH_SELECTION_FIELDS` to
// it, so that module imports it rather than this one.
use crate::resolve_hash::SelectionStateCanonical;

/// Re-exported so the public path `compiler::loader_api::RESOLVE_HASH_SELECTION_FIELDS`
/// is unchanged by the configflux-y2ai consolidation. The const itself belongs
/// beside the pre-image it describes; see `crate::resolve_hash`.
pub use crate::resolve_hash::RESOLVE_HASH_SELECTION_FIELDS;

/// registry: cause = the request's schema_version is not the version this build implements; remedy = set schema_version to the version this binary reports, or use a binary built for the version your caller targets
pub const E_LOADER_UNSUPPORTED_SCHEMA_VERSION: &str = "E_LOADER_UNSUPPORTED_SCHEMA_VERSION";
/// registry: cause = the compiled model package manifest could not be read or parsed as JSON, so the package cannot be opened; remedy = point the model handle at a manifest produced by a successful compile, and recompile the model if the file is damaged
pub const E_LOADER_MANIFEST_INVALID: &str = "E_LOADER_MANIFEST_INVALID";
/// registry: cause = the manifest parses but disagrees with the package it describes: an unsupported manifest version, a mismatched IR format, hash algorithm or model hash, or recorded statistics that do not match the index; remedy = recompile the model to regenerate a self-consistent package, and do not hand-edit a manifest or mix files from separate compilations
pub const E_LOADER_MANIFEST_INCONSISTENT: &str = "E_LOADER_MANIFEST_INCONSISTENT";
/// registry: cause = the package index failed to load, its recorded config hash does not match the hash computed from its contents, a chunk file it names is missing or modified, or the package declares a symbol that violates the authored-symbol rule; remedy = restore the complete, unmodified package directory or recompile the model: every chunk file the index names must be present and byte-identical
pub const E_LOADER_INDEX_INVALID: &str = "E_LOADER_INDEX_INVALID";
/// registry: cause = the supplied selection state is not consistent with the open model: its model hash or scope differs, its recorded state hash does not match its contents, or a choice contradicts a context tag; remedy = start from a fresh selection state initialized against the model you opened, and pass it back unmodified between calls
pub const E_SELECTION_STATE_INVALID: &str = "E_SELECTION_STATE_INVALID";
/// registry: cause = the requested facet name is blank, or no facet by that name is declared or discovered anywhere in the opened model; resolve raises it the same way for an implied_choices entry naming a facet or binding the model does not declare, and every selection operation raises it for a choice the supplied selection state already carries on such a facet; remedy = check the name for typos and list the model's facets first; a facet must exist in the compiled model before it can be selected
pub const E_SELECTION_UNKNOWN_FACET: &str = "E_SELECTION_UNKNOWN_FACET";
/// registry: cause = the facet exists, but the requested option is not a member of that facet's declared domain; resolve raises it the same way for an implied_choices entry carrying such a value, where a binding's domain is its catalogue's entry ids, and every selection operation raises it for a choice the supplied selection state already carries, or for a context tag on a facet whose options the model declares exhaustively; remedy = choose one of the options the diagnostic lists, or add the value to the facet's domain in the model source and recompile
pub const E_SELECTION_INVALID_OPTION: &str = "E_SELECTION_INVALID_OPTION";
/// registry: cause = the choice contradicts something already fixed: an immutable context tag, an earlier selection of the same facet, or a declared constraint the choice would violate. Also reported when the supplied selection state is one the solver proves unsatisfiable, whether or not the compiler can see the contradiction for itself; remedy = read the conflict the diagnostic names and drop or change the earlier choice; a constraint violation identifies the rule under an entity path of constraints/<id>; for a refused state, run explain with the same selection to see the minimal conflicting set
pub const E_SELECTION_CONFLICT: &str = "E_SELECTION_CONFLICT";
/// registry: cause = the option is valid on its own, but once applied no assignment of the remaining facets satisfies the model; remedy = query the valid options for the facet before choosing, or run explain to see the minimal set of choices that conflict
pub const E_SELECTION_UNSATISFIABLE: &str = "E_SELECTION_UNSATISFIABLE";
// ADR-0030 frozen codes (CCM hard precondition + fallback retirement). A usable
// `.ccm` solver model is required for the selection path; absence and internal
// faults fail closed instead of degrading to the legacy compiler path. Defined
// here, alongside the other selection/resolve codes, so both the interpreter
// (`interpreter::solver_session`) and the runtime (`runtime::solver_validation`)
// reference one canonical definition.
//
// Emitted when `options`/`select` cannot reach a usable solver model (empty
// reference, unloadable artifact, or symbol-less stub). ADR-0030 D1.
/// registry: cause = selection could not reach a usable solver model: the package's solver-model reference is empty, the artifact will not load, or it carries no symbol table; remedy = recompile the model so a complete solver model is emitted beside the package, and keep the two together whenever the package is copied or moved
pub const E_SELECTION_SOLVER_MODEL_UNAVAILABLE: &str = "E_SELECTION_SOLVER_MODEL_UNAVAILABLE";
// The selection surface's internal-fault family (ADR-0030), with exactly three
// meanings as narrowed by Amendment 2 (configflux-eclx): a solver-owned
// `options`/`select`/`set-parameter` query that faults internally (D4), a
// conflict the solver reports without a labeled core (ADR-0031 D3), and a
// `.ccm` that cannot hold a value the model sources DECLARE for a closed facet
// (Amendment 2 Rule 2's skew). All fail closed rather than degrade to legacy.
//
// It is NO LONGER what `select` reports when the two engines merely disagree.
// D3 assumed two complete engines over one formula; after ADR-0057 the
// compiler's check is three-valued and `Unknown` is never a violation
// (ADR-0054 §2), so over an unbound derive-only binding it legitimately accepts
// what the total BDD refuses. That case is now `E_SELECTION_CONFLICT`.
/// registry: cause = one of exactly three internal faults, none of them a usage error: the solver faulted while adjudicating the request, it reported a conflict without producing the minimal conflicting set, or the compiled model does not carry a value the model sources declare for a closed facet, which the message names; remedy = re-run with the same inputs to confirm. The third case is a stale compiled model and is fixed by recompiling; the other two are defects, to be reported with the model package and the exact sequence of selections
pub const E_SELECTION_ENGINE_DIVERGENCE: &str = "E_SELECTION_ENGINE_DIVERGENCE";
/// registry: cause = the scope selector could not be parsed, or names a form the resolver does not recognize; remedy = use a supported selector such as component:<id>, platform:<id>, platform:all, or all
pub const E_RESOLVE_SCOPE_INVALID: &str = "E_RESOLVE_SCOPE_INVALID";
/// registry: cause = the model could not be loaded for resolution, a declared constraint carries an expression the resolver cannot parse, or the package declares a symbol that violates the authored-symbol rule, so resolution fails closed rather than skipping the rule; remedy = recompile the model with the current toolchain, and correct any constraint expression the diagnostic names
pub const E_RESOLVE_MODEL_INVALID: &str = "E_RESOLVE_MODEL_INVALID";
/// registry: cause = an active condition references a facet or tag that nothing in the selection binds, so the condition cannot be evaluated; remedy = supply the missing facet as an explicit choice or as a context tag before resolving
pub const E_RESOLVE_CONTEXT_UNSATISFIED: &str = "E_RESOLVE_CONTEXT_UNSATISFIED";
// ADR-0047 §5: a DECLARED facet that has NO default is unbound after merging
// context_tags ∪ choices, yet an active condition needs it. Distinct from the
// generic `E_RESOLVE_CONTEXT_UNSATISFIED` fold: the model is satisfiable once
// the facet is bound, so this is a valid-input-but-underspecified USAGE error
// (cfx exit 2 per ADR-0042), and its message names the facet and its declared
// domain instead of the generic "unsatisfiable" hint.
/// registry: cause = a declared facet with no default is left unbound while an active condition needs it, or while a component in the resolved closure requires the binding it names, so the model is satisfiable but underspecified; remedy = bind the facet the diagnostic names to one of the values in its reported domain, or give that facet a default or a derive table in the model source; when the message lists requiring components, every one of them was waiting on the same binding
pub const E_RESOLVE_FACET_UNBOUND: &str = "E_RESOLVE_FACET_UNBOUND";
/// registry: cause = resolution failed for a reason outside the scope and context families, or the resolved output could not be canonically serialized for hashing; remedy = read the wrapped message for the underlying cause; report a serialization failure with the model package, since deterministic hashing must succeed
pub const E_RESOLVE_FAILED: &str = "E_RESOLVE_FAILED";
// Emitted when `resolve` cannot reach a usable solver model to gate
// satisfiability (absence, or a sat-gate fault). ADR-0030 D1/D4.
/// registry: cause = resolution could not reach a usable solver model to check satisfiability, or the solver faulted while replaying the committed choices; remedy = recompile the model so a complete solver model is emitted beside the package, and keep the two together when the package is moved
pub const E_RESOLVE_SOLVER_MODEL_UNAVAILABLE: &str = "E_RESOLVE_SOLVER_MODEL_UNAVAILABLE";
// ADR-0054 §6: the `entity_path` prefix a constraint-violation diagnostic
// carries. This is a MACHINE-CONSUMER CONTRACT, not cosmetics: §6 deliberately
// adds no new diagnostic code, so `constraints/<id>` on `entity_path` is the
// only way a consumer distinguishes a policy violation from the other
// `E_SELECTION_CONFLICT` causes (context-tag clash, already-selected facet).
pub const CONSTRAINT_ENTITY_PATH_PREFIX: &str = "constraints/";
/// registry: cause = the requested export profile is not one this build supports; remedy = use the early-binding profile named in the export contract; it is the only profile this version accepts
pub const E_EXPORT_PROFILE_INVALID: &str = "E_EXPORT_PROFILE_INVALID";
/// registry: cause = the resolve result handed to export is unusable: a wrong schema version, a status other than success, a missing resolve hash, or resolved output that cannot be decoded; remedy = pass the complete, unmodified result of a successful resolve rather than a hand-assembled or partially copied structure
pub const E_EXPORT_RESOLVE_INVALID: &str = "E_EXPORT_RESOLVE_INVALID";
/// registry: cause = a construction-lifecycle parameter typed as an artifact holds something other than a non-empty artifact identifier string; remedy = give the parameter the diagnostic names a valid artifact identifier in the model source, then recompile and resolve before exporting again
pub const E_EXPORT_ARTIFACT_INVALID: &str = "E_EXPORT_ARTIFACT_INVALID";
/// registry: cause = a generated C++ or CMake symbol is not a valid identifier, or two parameters generate the same symbol and would collide in the emitted header; remedy = rename the component or parameter the diagnostic names so the generated symbols are both valid and unique
pub const E_EXPORT_SYMBOL_INVALID: &str = "E_EXPORT_SYMBOL_INVALID";
/// registry: cause = the export artifacts could not be produced, most often because a resolved floating-point value is not finite and has no deterministic representation; remedy = replace any not-a-number or infinite value in the model with a finite one; report other failures with the resolved output that produced them
pub const E_EXPORT_FAILED: &str = "E_EXPORT_FAILED";
/// registry: cause = the requested software bill of materials profile is not one this build supports; remedy = use the full-audit profile to include resolved values, or the value-redacted profile to omit them
pub const E_SBOM_PROFILE_INVALID: &str = "E_SBOM_PROFILE_INVALID";
/// registry: cause = the supplied resolve result is unusable or self-contradictory: a wrong shape, empty resolved output, or a component or parameter carrying conflicting values across scope roots; remedy = pass the complete, unmodified result of a successful resolve, and resolve a scope whose roots agree on every shared component and parameter
pub const E_SBOM_RESOLVE_INVALID: &str = "E_SBOM_RESOLVE_INVALID";
/// registry: cause = a component in the bill of materials names a dependency that is not itself present in the document; remedy = resolve a scope that includes every component reachable through depends_on, so the dependency graph in the document is closed
pub const E_SBOM_PATH_INVALID: &str = "E_SBOM_PATH_INVALID";
/// registry: cause = an artifact-typed parameter holds a blank or non-string value, or names an artifact that is absent from the resolved artifact catalog; remedy = ensure the resolve result you pass carries every artifact its parameters reference, then regenerate the document
pub const E_SBOM_ARTIFACT_INVALID: &str = "E_SBOM_ARTIFACT_INVALID";
/// registry: cause = a parameter in the resolved output declares a lifecycle outside the supported set; remedy = give every parameter one of the supported lifecycles: construction, startup, or runtime
pub const E_SBOM_BINDING_INVALID: &str = "E_SBOM_BINDING_INVALID";
/// registry: cause = a document's recorded component, parameter, or artifact counts disagree with its own contents, which signals tampering or corruption rather than bad input; remedy = regenerate the document from a fresh resolve; a stored bill of materials must never be hand-edited, because its counts and hash are part of its evidence value
pub const E_SBOM_STATS_INVALID: &str = "E_SBOM_STATS_INVALID";
/// registry: cause = a stored document fails verification: its hash algorithm, canonicalization version, document version, or recorded hash does not match its contents; remedy = regenerate the document; a mismatch means the stored bytes changed after the document was produced, so the copy in hand cannot be trusted as evidence
pub const E_SBOM_HASH_INVALID: &str = "E_SBOM_HASH_INVALID";
/// registry: cause = the assembled bill of materials could not be canonically serialized, so its content hash could not be computed; remedy = report this with the resolve result used, since canonical serialization is expected to succeed for every well-formed document
pub const E_SBOM_FAILED: &str = "E_SBOM_FAILED";

pub const EXPORT_PROFILE_CPP_EARLY_BINDING_V1: &str = "cpp_early_binding_v1";
pub const EXPORT_SOFTWARE_BOM_PROFILE_FULL_AUDIT: &str = "full_audit";
pub const EXPORT_SOFTWARE_BOM_PROFILE_VALUE_REDACTED: &str = "value_redacted";
pub const GENERATED_CONFIG_HPP_PATH: &str = "generated/config.hpp";
pub const GENERATED_CONFIG_BUILD_FLAGS_PATH: &str = "generated/config_build_flags.cmake";
pub const GENERATED_CONFIG_ARTIFACT_MANIFEST_PATH: &str = "generated/config_artifact_manifest.json";
pub const SOFTWARE_BOM_HASH_ALGO: &str = "sha256";
pub const SOFTWARE_BOM_CANONICALIZATION_VERSION: u32 = 1;
pub const SOFTWARE_BOM_VERSION: u32 = 1;
pub const SOFTWARE_BOM_GENERATOR_NAME: &str = "configflux-sbom";
pub const SOFTWARE_BOM_GENERATOR_VERSION: &str = "0.1.0";
pub const SOFTWARE_BOM_GENERATED_AT_RFC3339: &str = "1970-01-01T00:00:00Z";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionState {
    pub schema_version: u32,
    pub model_hash: String,
    pub scope: String,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    pub selection_state_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionDelta {
    pub facet: String,
    pub option: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrunedOptionReason {
    pub option: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSelectionOptionsRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
    pub facet: String,
    #[serde(default)]
    pub include_pruned_reasons: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GetSelectionOptionsResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    pub facet: String,
    pub valid_options: Vec<String>,
    // ADR-0047 §6: the facet's declared default arm, when the facet is a
    // first-class declaration that carries one. Skip-if-none so undeclared /
    // default-less facets — and every pre-ADR-0047 model — emit no key and stay
    // byte-identical. `cfx options` renders it as `[default: <value>]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    // ADR-0047 §6 (Amendment 1): the facet's declared domain-openness —
    // `Some(false)` closed, `Some(true)` open — for a first-class declared
    // facet; `None` for an undeclared facet (no schema kind to report). Additive
    // and skip-if-none, so undeclared-facet output and every pre-ADR-0047 model
    // stay byte-identical. `cfx options` renders it as the `[closed]`/`[open]`
    // schema-kind token, distinct from the selection-state marker, so a declared
    // closed facet no longer mislabels as `[open]` merely because it is
    // unselected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pruned_options: Option<Vec<PrunedOptionReason>>,
    pub selection_state_hash: String,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplySelectionRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
    pub selection_delta: SelectionDelta,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplySelectionResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_state: Option<SelectionState>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainRejectionRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
    pub rejected_option: SelectionDelta,
}

/// One labeled `{facet}.{option}` pair inside an unsat core. Used both for the
/// `rejected` selection being explained and for each facet/option named by a
/// conflicting constraint. Mirrors the ADR-0031 D3 `{ facet, option }` shape.
/// Labeled names only — never raw BDD variable indices (ADR-0031 D3 invariant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ConstraintFacet {
    pub facet: String,
    pub option: String,
}

/// Whether a conflicting constraint is a prior *selection* the caller already
/// made (a choice in `selection_state`) or a *model rule* baked into the `.ccm`
/// (a `requires`/`excludes`-style constraint). ADR-0031 D3 `kind` field.
///
/// Serializes snake_case (`"selection"` / `"model_rule"`) to match the stable
/// JSON schema. The compiler only serializes this enum; the solver decides
/// which variant each MUS clause is (ADR-0031 D3, solver-decided content).
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstraintKind {
    Selection,
    ModelRule,
}

/// One entry in the labeled minimal unsatisfiable subset (MUS): a single
/// constraint that, together with the `rejected` selection, contributes to the
/// unsatisfiability. ADR-0031 D3 `conflicting_constraints[]` element.
///
/// `summary` is advisory human-gloss text, not a parsed field (ADR-0031 D3).
///
/// `constraint_id` names the **authored** `constraints:` entry this clause is
/// attributed to (ADR-0054 §5.4), which is the field a machine consumer reads
/// to tie a conflict back to declared policy. It is `None` — and omitted from
/// the JSON — for a `Selection` (a prior choice is not a declared constraint)
/// and for a `ModelRule` that no declared constraint accounts for. That second
/// case means the *model* is over-constrained, not that the user violated a
/// policy: synthesized intra-facet cardinality conjuncts are deliberately
/// absent from the roster and must never be named here as if they were
/// authored policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictingConstraint {
    pub kind: ConstraintKind,
    pub facets: Vec<ConstraintFacet>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constraint_id: Option<String>,
}

/// The labeled unsat core attached to a genuine constraint-conflict rejection
/// (`E_SELECTION_CONFLICT` / `E_SELECTION_UNSATISFIABLE`). ADR-0031 D3.
///
/// The solver produces the decision content (the labeled MUS); the compiler
/// serializes these envelope bytes. The interpreter/runtime wrappers convert
/// the solver-owned core type into this compiler-side type — the compiler never
/// imports the solver (ADR-0003 §2). `conflicting_constraints` MUST contain
/// only labeled `{facet}.{option}` names, never raw BDD variable indices
/// (ADR-0031 D3 schema invariant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnsatCore {
    pub rejected: ConstraintFacet,
    pub conflicting_constraints: Vec<ConflictingConstraint>,
    pub minimal: bool,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RejectionReason {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub blocking_choices: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// The labeled unsat core (ADR-0031 D3). Present only on solver-decided
    /// constraint-conflict rejections (`E_SELECTION_CONFLICT` /
    /// `E_SELECTION_UNSATISFIABLE`); `None`/omitted for the division-of-labor
    /// rejections the compiler owns without a solver core (ADR-0030 D5) and for
    /// fail-closed command errors (ADR-0031 D4). Additive `Option` field so the
    /// existing constructors and all prior callers degrade cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unsat_core: Option<UnsatCore>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExplainRejectionResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    pub facet: String,
    pub option: String,
    pub rejection: RejectionReason,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolveFromSelectionRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    pub selection_state: SelectionState,
    // ADR-0057 §D6: facets the SOLVER determined the constraints already decide,
    // supplied by `session_compose::resolve` — the one seam permitted to consult
    // the solver (ADR-0003 §2). The compiler seeds the tag environment in the
    // order declared defaults, implied, context tags, choices, so this sits
    // ABOVE a default and BELOW anything the user actually stated.
    //
    // `#[serde(default)]` keeps every pre-ADR-0057 request valid, and a
    // compiler-direct caller that passes nothing gets today's behavior exactly —
    // which is what lets the resolver stay solver-free while still honoring an
    // inference it did not make.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub implied_choices: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolveResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    pub selection_state_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_hash: Option<String>,
    // ADR-0059 D3: the identity of the DELIVERED PAYLOAD alone. `resolve_hash`
    // folds `model_hash` into its pre-image, so it rotates on any model edit —
    // including one that leaves the bytes a service receives byte-identical.
    // This hash covers `{schema_version, scope, resolved_output}` and nothing
    // else, which is what makes "did my change touch this deployment"
    // answerable by hash comparison. Declared HERE, immediately after
    // `resolve_hash`, so the two lineage fields serialize side by side.
    // Present iff `resolved_output` is present; additive and outside every
    // existing pre-image, so no `schema_version` bump (ADR-0059 D3, M3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_output_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub choices: BTreeMap<String, String>,
    // ADR-0047 §5: provenance for auto-bound declared-facet defaults. Records
    // exactly the declared facets whose resolved value came from the declared
    // default seed (i.e. were NOT overridden by a context tag or explicit
    // choice). Skip-if-empty (mirroring `context_tags`/`choices`) so a model
    // with no declared facets — or none that defaulted — emits no key and its
    // `resolve_hash` pre-image is byte-unchanged by this feature.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub defaulted_choices: BTreeMap<String, String>,
    // ADR-0057 §D6: provenance for the facets the SOLVER determined the
    // constraints already decide — the facet's domain had collapsed to a single
    // value once the user's tags and choices were applied. Declared immediately
    // after `defaulted_choices` because the two are the resolve-time provenance
    // PAIR: together they answer "where did this facet's value come from" for
    // every facet the user did not state. A facet recorded here is NEVER also
    // recorded in `defaulted_choices` — the model had an opinion, so the default
    // never applied.
    //
    // Folded into `ResolveHashCanonical` with the same skip-if-empty rule
    // `defaulted_choices` carries, and for the same reason: it is provenance
    // about THIS resolve's selection, so two resolves that differ only in what
    // the model forced must not collide. Skip-if-empty keeps every model where
    // nothing is implied byte-identical.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub implied_choices: BTreeMap<String, String>,
    // ADR-0060 §6 route (a): the model's CLOSED facet declarations, recorded
    // here because the resolver is the last party that holds a `ModelHandle`.
    // A device carries a `.ccm` and a `resolve.result.json`, never the chunk
    // set, so this envelope is the ONLY channel by which closed-ness reaches
    // the runtime — where it lets an explain/rejection core that mentions a
    // closed facet only negatively still name the constraint it breaks
    // (configflux-pt6v, configflux-tkwt). Declared immediately AFTER the two
    // resolve-time provenance maps (`defaulted_choices` and, since ADR-0057 §D6,
    // `implied_choices`) and deliberately not between them: it is not one of
    // them. ADR-0060's own text called it a third "resolve-time provenance
    // field" in its migration list while arguing the opposite everywhere else;
    // the paragraph below is the operative reading (configflux-secb.3).
    // Skip-if-empty keeps every facet-free model's envelope byte-identical.
    //
    // NOT a lineage field, and it MUST NOT be folded into
    // `ResolveHashCanonical`: unlike `defaulted_choices` it says nothing about
    // the user's selection, only about static declarations `model_hash`
    // already covers. Folding it in would rotate `resolve_hash` on every model
    // with a closed facet — the hard stop ADR-0060 is required to hold. It is a
    // sibling of both pre-images and a member of neither, exactly as ADR-0059
    // D3 framed `resolved_output_hash`.
    #[serde(default, skip_serializing_if = "ClosedFacetDomains::is_empty")]
    pub closed_facet_domains: ClosedFacetDomains,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_component_dependencies: BTreeMap<String, BTreeMap<String, Vec<String>>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub resolved_artifacts: BTreeMap<String, crate::schema::Artifact>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportResolvedRequest {
    pub schema_version: u32,
    pub resolve_result: ResolveResult,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedArtifact {
    pub path: String,
    pub contents: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GeneratedArtifacts {
    pub profile: String,
    pub generator_hash: String,
    pub files: Vec<GeneratedArtifact>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportResolvedResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_artifacts: Option<GeneratedArtifacts>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
    /// Tool-identity stamp (ADR-0044 D1 / `configflux-pq2w.1`). This is an
    /// ENVELOPE-ONLY builder (no output dir → no sidecar file), so identity
    /// rides this additive optional field instead. Never enters `resolve_hash`
    /// (computed over `SelectionState`/resolved output, not this envelope), so
    /// adding it changes no hash and implies no `schema_version` bump.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_version: Option<String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoftwareBomBindingPhase {
    Early,
    Late,
    Runtime,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomGeneratorMetadata {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomComponentEntry {
    pub component_id: String,
    pub r#type: String,
    pub dependency_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoftwareBomParameterEntry {
    pub path: String,
    pub component_id: String,
    pub param_key: String,
    pub r#type: String,
    pub value: crate::schema::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    pub safety: crate::schema::SafetyLevel,
    pub lifecycle: crate::schema::Lifecycle,
    pub binding_phase: SoftwareBomBindingPhase,
    pub access: crate::schema::Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub req_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<crate::schema::Limits>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomArtifactEntry {
    pub artifact_id: String,
    pub bound_paths: Vec<String>,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareBomStats {
    pub component_count: u32,
    pub parameter_count: u32,
    pub artifact_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SoftwareBomV1 {
    pub schema_version: u32,
    pub bom_version: u32,
    pub bom_hash: String,
    pub hash_algo: String,
    pub canonicalization_version: u32,
    pub model_hash: String,
    pub resolve_hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_state_hash: Option<String>,
    pub scope_root: String,
    pub generated_at: String,
    pub generator: SoftwareBomGeneratorMetadata,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
    #[serde(default)]
    pub choices: BTreeMap<String, String>,
    pub components: Vec<SoftwareBomComponentEntry>,
    pub parameters: Vec<SoftwareBomParameterEntry>,
    pub artifacts: Vec<SoftwareBomArtifactEntry>,
    pub stats: SoftwareBomStats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSoftwareBomRequest {
    pub schema_version: u32,
    pub resolve_result: ResolveResult,
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportSoftwareBomResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolve_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bom_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software_bom: Option<SoftwareBomV1>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
    /// Tool-identity stamp (ADR-0044 D1 / `configflux-pq2w.1`). Envelope-only
    /// builder (no output dir → no sidecar file); identity rides this additive
    /// optional field. The hashed SBOM `generator`/`generated_at` stay frozen
    /// (D1.5) — this field lives on the RESULT envelope, NOT inside
    /// `SoftwareBomV1`, so `bom_hash` is unchanged and no `schema_version`
    /// bump is implied.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_version: Option<String>,
}

/// ADR-0059 D3 pre-image for `resolved_output_hash`. Field order is the
/// contract and is asserted by `resolved_output_hash_preimage_is_pinned`.
///
/// It DELIBERATELY excludes `model_hash`, `selection_state`, `context_tags`,
/// `choices`, and `defaulted_choices`. That exclusion is the entire point: it
/// identifies the bytes a service receives and nothing else, which is the
/// question `resolve_hash` structurally cannot answer. Two resolves of two
/// DIFFERENT models that deliver the same payload agree here and differ there.
///
/// Every field is mandatory — there is no skip-if-empty escape hatch — because
/// unlike `ResolveHashCanonical` this pre-image has no pre-existing pinned
/// literals to keep byte-stable.
#[derive(Debug, Serialize)]
struct ResolvedOutputHashCanonical<'a> {
    schema_version: u32,
    scope: &'a str,
    resolved_output: &'a serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct GeneratorHashCanonical<'a> {
    schema_version: u32,
    profile: &'a str,
    model_hash: &'a str,
    scope: &'a str,
    resolve_hash: &'a str,
    files: &'a [GeneratedArtifactHashCanonical<'a>],
}

#[derive(Debug, Clone, Serialize)]
struct GeneratedArtifactHashCanonical<'a> {
    path: &'a str,
    content_hash: &'a str,
}

#[derive(Debug, Clone)]
struct ConstructionParamBinding {
    entity_path: String,
    value: crate::schema::Value,
    header_symbol: String,
    cmake_var: String,
    compile_definition: String,
}

#[derive(Debug, Clone, Serialize)]
struct ArtifactManifest {
    schema_version: u32,
    profile: String,
    model_hash: String,
    resolve_hash: String,
    artifacts: Vec<ArtifactManifestEntry>,
}

#[derive(Debug, Clone, Serialize)]
struct ArtifactManifestEntry {
    artifact_id: String,
    bound_paths: Vec<String>,
}

#[derive(Debug, Clone)]
struct EarlyBindingExtraction {
    construction_bindings: Vec<ConstructionParamBinding>,
    artifact_manifest_entries: Vec<ArtifactManifestEntry>,
}

#[derive(Debug, Clone)]
struct ExportGenerationError {
    code: &'static str,
    message: String,
    entity_path: Option<String>,
    hint: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SoftwareBomGenerationError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
    pub(crate) entity_path: Option<String>,
    pub(crate) hint: Option<String>,
}

#[derive(Debug, Default, Clone)]
struct SelectionConstraintModel {
    facet_domains: BTreeMap<String, BTreeSet<String>>,
    conditions: Vec<ConditionExpr>,
    // ADR-0047 §4: the authored (declared) values of first-class facets, keyed
    // by facet. Distinct from `facet_domains` (which unions declared values
    // with condition-inferred ones): only DECLARED values live here, so the
    // option-validity check can accept a declared value — including a default
    // arm that no condition names — while leaving the legacy
    // condition-inferred rule byte-for-byte unchanged for undeclared facets
    // (this map is empty for any model that declares no facet).
    declared_values: BTreeMap<String, BTreeSet<String>>,
    // ADR-0047 §5/§6: a declared facet's declared default (when it has one),
    // keyed by facet. Seeded from the declarations alongside `declared_values`.
    // `cfx options` reads it to annotate the facet's default arm; resolve reads
    // it to auto-bind. Empty for any model that declares no defaulted facet.
    facet_defaults: BTreeMap<String, String>,
    // ADR-0047 §6 (Amendment 1): a declared facet's domain-openness (`false`
    // closed, `true` open), keyed by facet. Seeded from the declarations
    // alongside `declared_values`. `cfx options` reads it to render the truthful
    // `[closed]`/`[open]` schema-kind token. Empty (⇒ `declared_open: None`) for
    // any undeclared facet and every model that declares no facet.
    facet_open: BTreeMap<String, bool>,
    // ADR-0054 §4: the model's authored policy assertions, in id-ascending
    // order.
    //
    // THIS IS NOT `conditions`, AND THE TWO MUST NEVER BE MERGED INTO ONE LIST
    // AGAIN (ADR-0054 §3). They are different kinds of thing that happen to
    // share a grammar:
    //
    //   * `conditions` holds INCLUSION SELECTORS harvested from component,
    //     parameter, and override `condition` fields. A selector decides what a
    //     resolved configuration CONTAINS — it is the 150% -> 100% filter — and
    //     asserts nothing about which selections are legal.
    //   * `constraints` holds POLICY ASSERTIONS. Each one decides what a user is
    //     ALLOWED TO PICK, under one rule: every declared constraint must hold
    //     in every resolved configuration.
    //
    // Conflating them is the defect ADR-0054 exists to end: it is what let a
    // branch selector prune an option model-wide (configflux-9xxq) and what let
    // a policy written as a phantom component be enforced on two of the three
    // public surfaces and ignored on the third (configflux-4sjk). A constraint
    // also carries an ID, which a selector has no notion of and which
    // `cfx explain` needs to name a violated policy.
    //
    // Deliberately, a constraint does NOT widen `facet_domains`: a policy
    // asserts over a domain, it never creates one. `link_verify::
    // validate_constraints` enforces every facet named here is DECLARED, not
    // merely condition-inferred — ADR-0054 §5.2 amendment, configflux-6j91.
    constraints: Vec<SelectionConstraint>,
}

/// One authored policy assertion, as the SELECTION surfaces need it.
///
/// `resolve_from_selection` reads constraints straight off the merged
/// `Config` (`ResolveModel`), where the authored `condition` text and the
/// declaring chunk are both still to hand. The selection path loads a much
/// narrower model and used to keep only `(id, expr)` — enough to DECIDE a
/// violation, not enough to REPORT one. Carrying the text and the source id
/// here is what lets a rejected `apply_selection` render exactly the ADR-0054
/// §6 diagnostic `resolve` renders, instead of a second, vaguer message for
/// the same policy (configflux-narb).
#[derive(Debug, Clone)]
struct SelectionConstraint {
    /// The authored constraint id — what `cfx explain` and the §6 diagnostic
    /// name.
    id: String,
    /// The authored condition text, quoted verbatim in the §6 diagnostic. The
    /// parsed `expr` cannot be printed back as the author wrote it.
    condition: String,
    /// The parsed assertion, evaluated by `not_contradicted` against a
    /// (possibly partial) assignment.
    expr: ConditionExpr,
    /// The `source_id` of the chunk that declared it (ADR-0054 §6).
    source_id: String,
}

/// What `load_resolve_model` hands `resolve_from_selection`: the merged
/// authored model, plus the provenance the merge would otherwise throw away.
///
/// `Config` is the AUTHORED schema shape — it has no notion of which file an
/// entity came from, and it must not grow one (it is what a user writes, and
/// what `constraints` passes through verbatim per ADR-0054 §1). But ADR-0054
/// §6's rejection diagnostic specifies `source_id` = "the chunk that declared
/// the constraint", so the loader records it beside the config rather than
/// inside it (configflux-emmg).
///
/// The map is filled on the chunk walk `load_resolve_model` ALREADY performs,
/// from the `IrChunkRef.source_id` that loop ALREADY holds — no second pass
/// over the package, and nothing is re-read on the error path.
#[derive(Debug, Clone)]
struct ResolveModel {
    config: crate::schema::Config,
    /// Constraint id → the `source_id` of the chunk that declared it. Total
    /// over `config.constraints` (ingest rejects duplicate ids across chunks,
    /// so the mapping is a function), and over `lowered_conjuncts` — where the
    /// declaring chunk is the one that declared the binding or the component
    /// the conjunct came from.
    constraint_sources: BTreeMap<String, String>,
    /// The ADR-0057 §D4 lowered root conjuncts — `(attribution_id, condition)`
    /// — in the emitter's fold order: `derive:` conjuncts first, then
    /// `accepts:`.
    ///
    /// Beside `config`, not inside `config.constraints`, for the same reason
    /// `constraint_sources` is: `Config` is the AUTHORED shape a user writes,
    /// and `derive`/`accepts` are authored on a binding and a component. Mixing
    /// synthesized entries into the authored map would make every reader of
    /// `config.constraints` — the snapshot, the inspect surface, a future
    /// round-trip — report rules nobody wrote under that namespace.
    ///
    /// Evaluated by `resolve_ops::evaluate_constraints` after the authored
    /// constraints, so a resolve rejects on exactly the conjuncts the `.ccm`
    /// asserts and names them by the same ids `cfx explain` does. Empty for
    /// every model that declares no `derive` table and no `accepts` list.
    lowered_conjuncts: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenModelRequest {
    pub schema_version: u32,
    pub cmp_manifest_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelHandle {
    pub model_hash: String,
    pub cmp_manifest_ref: String,
    pub index_ref: String,
    pub chunk_set_ref: String,
    /// Path to the sibling `.ccm` artifact directory emitted alongside the
    /// CMP package (configflux-9hi2). Downstream callers (`solver::Session::
    /// load_ccm`, ADR-0017 §2) locate and load the v2 multi-part `.ccm` here.
    /// Resolved as `<cmp_manifest_dir>/ccm`. Defaults to empty for handles
    /// produced before this field existed; consumers treat empty as
    /// "no sibling `.ccm` advertised".
    #[serde(default)]
    pub ccm_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenModelResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_handle: Option<ModelHandle>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeSelectionStateRequest {
    pub schema_version: u32,
    pub model_handle: ModelHandle,
    pub scope: String,
    #[serde(default)]
    pub context_tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializeSelectionStateResult {
    pub schema_version: u32,
    pub status: OperationStatus,
    pub model_hash: String,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_state: Option<SelectionState>,
    pub error_count: u32,
    pub warning_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics_ref: Option<String>,
    pub diagnostics: DiagnosticsReport,
}
