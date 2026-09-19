// SPDX-License-Identifier: BUSL-1.1

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

// ============================================================================
// 1. Core Enums
// ============================================================================

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Developer,
    Integrator,
    Technician,
    Supervisor,
    SuperUser,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum SafetyLevel {
    QM,   // Quality Managed (Standard)
    Sil1, // Safety Integrity Level 1
    Sil2,
    Sil3,
    Sil4,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Construction, // Compile-time constant
    Startup,      // Read-only after boot
    Runtime,      // Mutable
}

// ============================================================================
// 2. Value System
// ============================================================================

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
#[serde(untagged)]
pub enum Value {
    Integer(i64),
    Float(f64),
    Boolean(bool),
    String(String),
    // Note: Expressions (e.g., "${x} + 1") are currently parsed as String.
    // The Resolver will eventually identify them.
}

// ============================================================================
// 3. Constraints
// ============================================================================

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Limits {
    pub min: Option<Value>,
    pub max: Option<Value>,
    pub min_len: Option<usize>,
    pub max_len: Option<usize>,
}

// ============================================================================
// 4. Recursive Override Blocks
// ============================================================================

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct ConditionalBlock {
    pub condition: String,

    // The Payload is a full Parameter struct, flattened.
    // This allows nested overrides (recurtion).
    #[serde(flatten)]
    pub payload: Box<Parameter>,
}

// ============================================================================
// 5. The Parameter (Source of Truth)
// ============================================================================

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Parameter {
    // --- Inheritance ---
    pub inherits: Option<String>,

    // --- Identity & Semantics ---
    pub r#type: Option<String>,
    pub unit: Option<String>,
    pub doc: Option<String>,

    // --- Data ---
    pub value: Option<Value>,

    /// The facet this parameter IS the runtime handle for (ADR-0064 D1).
    ///
    /// A parameter is a facet's handle ONLY when it declares so: name
    /// coincidence binds nothing. When present, `link_verify` checks the
    /// declaration (the facet is declared, the effective type is `string`, the
    /// parameter authors no value of its own, and at most one parameter in the
    /// model binds a given facet) and the resolver makes the parameter's value
    /// the facet's effective value.
    ///
    /// `skip_serializing_if` is load-bearing for the same reason it is on
    /// [`Component::requires`]: this field is inside the `chunk_hash` preimage
    /// (`ir::ChunkPreimage`), so an always-present `"facet": null` would rotate
    /// the `model_hash` of every model in existence. Omitting it keeps the
    /// addition hash-neutral for every model that declares no binding, which is
    /// what lets the committed golden corpus stay untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,

    // --- System Behavior ---
    pub lifecycle: Option<Lifecycle>,
    pub safety: Option<SafetyLevel>,
    pub access: Option<Role>,

    // --- Constraints ---
    pub limits: Option<Limits>,
    pub req_id: Option<String>,

    // --- The 150% Model (Recursive Overrides) ---
    #[serde(default)]
    pub overrides: Vec<ConditionalBlock>,
}

// ============================================================================
// 6. Root Structures
// ============================================================================

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Component {
    // Optional during ingestion to allow overlays to omit type; enforced before resolution.
    pub r#type: Option<String>,
    // If condition evaluates to false, component is removed from output
    pub condition: Option<String>,
    // Explicit component dependencies (compile-time validation).
    #[serde(default)]
    pub depends_on: Vec<String>,
    /// What this component NEEDS from the model's `bindings`, keyed by slot
    /// (ADR-0057 §D4). A requirement is the ONLY way a component receives a
    /// catalogue entry, so a component that forgets to declare its need has
    /// nothing to read and the mistake cannot be silent.
    ///
    /// A requirement is deliberately NOT a `depends_on` edge: it names a shared
    /// CHOICE, not another component, so it neither joins the dependency
    /// closure nor constrains build order.
    ///
    /// `skip_serializing_if` is load-bearing, not cosmetic. This map is inside
    /// the `chunk_hash` preimage (`ir::ChunkPreimage`, reached from a `Config`
    /// by `ir::chunk_hash_from_config` and from an emitted chunk by
    /// `ir::chunk_hash_of_chunk`), so an always-present `"requires": {}` would
    /// rotate the `model_hash` of every model in existence — including the ones
    /// that will never declare a requirement. Omitting the empty map keeps this
    /// addition hash-neutral for models that do not use it, which is what lets
    /// the committed golden corpus stay untouched.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requires: BTreeMap<String, Requirement>,
    #[serde(default)]
    pub params: HashMap<String, Parameter>,
}

/// One component requirement: the binding a slot needs, and optionally the
/// subset of that binding's catalogue entries the component can actually work
/// with (ADR-0057 §D4).
///
/// Two authored forms, one type. The bare form names the binding and accepts
/// every entry:
///
/// ```json
/// "requires": { "container": "line_container" }
/// ```
///
/// The explicit form narrows it:
///
/// ```json
/// "requires": { "container": { "binding": "line_container", "accepts": ["c1", "c2"] } }
/// ```
///
/// The bare form deserializes to `accepts: None`, which means "all entries" and
/// is NOT the same as `Some(vec![])` — an empty list is an authoring error
/// (`E_REQUIRES_INVALID`), because a component that accepts nothing can never
/// be satisfied.
///
/// Data shape only (ADR-0021 "CUE authors, Rust re-validates"). Requirement
/// resolution, `accepts` membership, and the cross-component
/// empty-intersection check all live in
/// [`crate::link_verify::validate_requirements`], over the merged
/// [`crate::interface_summary::MergedSummary`]. The lowering of `accepts` into
/// a root conjunct lives in [`crate::lowering`].
#[derive(Debug, Serialize, PartialEq, Eq, Clone)]
pub struct Requirement {
    /// The binding this slot is bound to. Must name a declared binding
    /// (`E_REQUIRES_INVALID`).
    pub binding: String,
    /// The catalogue entries this component can work with. `None` means every
    /// entry; `Some(list)` must be non-empty, free of duplicates, and a subset
    /// of the binding's catalogue entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts: Option<Vec<String>>,
}

impl<'de> Deserialize<'de> for Requirement {
    /// Accept both authored forms (see the type docs). Written as an untagged
    /// shim rather than a hand-rolled visitor so the object arm keeps serde's
    /// own field handling — both the authored JSON exported from CUE and the
    /// TOML backstop are self-describing, which is all `untagged` requires.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Form {
            Bare(String),
            Explicit {
                binding: String,
                #[serde(default)]
                accepts: Option<Vec<String>>,
            },
        }

        Ok(match Form::deserialize(deserializer)? {
            Form::Bare(binding) => Requirement {
                binding,
                accepts: None,
            },
            Form::Explicit { binding, accepts } => Requirement { binding, accepts },
        })
    }
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Config {
    pub package: String,
    pub version: String,
    #[serde(default)]
    pub definitions: HashMap<String, Parameter>,
    #[serde(default)]
    pub components: HashMap<String, Component>,
    #[serde(default)]
    pub artifacts: HashMap<String, Artifact>,
    // Fourth top-level namespace: authored facet/domain declarations (ADR-0047).
    // A facet is a named, ordered value domain with an optional default. Opt-in
    // per facet — undeclared facets keep the legacy condition-inferred domain.
    #[serde(default)]
    pub facets: HashMap<String, Facet>,
    // Fifth top-level namespace: authored policy assertions (ADR-0054 §1). A
    // constraint is a NAMED propositional formula over facet values; the one
    // rule is "every declared constraint must hold in every resolved
    // configuration". Pack-global — no inheritance, no gap-fill, no merge —
    // exactly like `facets`.
    #[serde(default)]
    pub constraints: HashMap<String, Constraint>,
    // Sixth top-level namespace: typed catalogues (ADR-0057 §D2). A catalogue is
    // a table of named entries, each supplying every declared field with a
    // value of the declared type. Pack-global and declared by at most one
    // chunk, exactly like `facets` — no inheritance, no gap-fill, no merge.
    #[serde(default)]
    pub catalogues: HashMap<String, Catalogue>,
    // Seventh top-level namespace: bindings (ADR-0057 §D3). A binding is ONE
    // shared choice of a catalogue entry, and semantically it IS a declared
    // closed facet whose domain is the catalogue's entry ids — so it shares
    // the facet id space and nothing downstream special-cases it.
    #[serde(default)]
    pub bindings: HashMap<String, Binding>,
}

// ============================================================================
// 6b. Constraints (ADR-0054)
// ============================================================================

/// An authored policy assertion: a named propositional formula over facet
/// values, written in the EXISTING condition grammar (`conditions::
/// parse_condition_expr` -> `ConditionExpr`). ADR-0054 §1 introduces no new
/// expression language, operator, or evaluator.
///
/// A constraint is categorically distinct from a `condition` on a component,
/// parameter, or override (ADR-0054 §3): those are inclusion *selectors* that
/// decide what a configuration contains, and nothing else. A constraint decides
/// what a user is allowed to pick. The two are never merged.
///
/// Data shape only. The invariants live in `link_verify::validate_constraints`
/// (ADR-0021 "CUE authors, Rust re-validates"), where an expression that does
/// not parse is a hard ingest ERROR — unlike a selector condition, which widens
/// no facet on a parse failure. A policy that cannot be understood must never be
/// silently dropped.
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Constraint {
    /// A Boolean expression over facet values, in the existing condition
    /// grammar. MUST parse.
    pub condition: String,
    pub doc: Option<String>,
}

// ============================================================================
// 6a. Facets (ADR-0047)
// ============================================================================

/// An authored facet declaration: a named, ordered value domain with an
/// optional default. Data shape only — the invariants CUE enforces (`values`
/// non-empty and unique, `default` a member of `values`) are re-validated in
/// Rust at ingest (ADR-0021 "CUE authors, Rust re-validates"); the closed-vs-
/// open condition-value check lives in `link_verify`.
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Facet {
    /// Ordered, non-empty, unique value domain (declared order is significant
    /// for symbol emission and BDD layout — ADR-0047 §4).
    pub values: Vec<String>,
    /// The default arm; when present it must be an element of `values`.
    pub default: Option<String>,
    /// `false` = closed (exhaustive) domain; `true` = extensible. Absent in the
    /// authored JSON when false (CUE default), so `serde(default)` restores it.
    #[serde(default)]
    pub open: bool,
    pub doc: Option<String>,
}

// ============================================================================
// 6c. Catalogues and bindings (ADR-0057 §D2, §D3)
// ============================================================================

/// The declared value type of one catalogue field.
///
/// Deliberately a closed enum rather than the free-form `Parameter::type`
/// string: a catalogue is a *typed table*, and the point of declaring a field's
/// type is that every entry can be checked against it at ingest
/// (`link_verify::validate_catalogues`). A free string would push that check
/// out to whoever reads the entry, which is exactly the late failure ADR-0057
/// exists to remove.
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "snake_case")]
pub enum CatalogueFieldType {
    Integer,
    Float,
    Boolean,
    String,
}

impl CatalogueFieldType {
    /// The authored spelling, for diagnostics.
    pub fn as_str(self) -> &'static str {
        match self {
            CatalogueFieldType::Integer => "integer",
            CatalogueFieldType::Float => "float",
            CatalogueFieldType::Boolean => "boolean",
            CatalogueFieldType::String => "string",
        }
    }
}

/// One column of a catalogue: a declared type plus the same optional
/// `unit`/`doc` semantics a `Parameter` carries, so a catalogue field means the
/// same thing to a reader as a parameter does.
///
/// Data shape only (ADR-0021 "CUE authors, Rust re-validates"); the invariants
/// live in `link_verify::validate_catalogues`.
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct CatalogueField {
    pub r#type: CatalogueFieldType,
    pub unit: Option<String>,
    pub doc: Option<String>,
}

/// A typed table of named entries (ADR-0057 §D2): the physical containers a
/// plant uses, the firmware modes a device supports, the motor variants a line
/// can be built from.
///
/// `fields` and `entries` are `BTreeMap`s on purpose. The authored form is a
/// CUE struct exported to a JSON object and parsed into a serde map, and this
/// repository's canonical JSON deliberately does not preserve object order
/// (`ir::chunk_hash_from_config` depends on that). "Declaration order" is
/// therefore id-ascending order — the only order the wire format carries —
/// and it is what a binding's value domain, the emitted `.ccm` symbols, and
/// `cfx options` all commit to.
///
/// Data shape only. Non-empty `fields`, non-empty `entries`, entry
/// completeness/exactness and per-field type agreement are re-validated at link
/// time in `link_verify::validate_catalogues` (`E_CATALOGUE_INVALID`); the
/// at-most-one-declaring-chunk rule is enforced at ingest merge
/// (`E_INGEST_DUPLICATE_CATALOGUE`).
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Catalogue {
    /// Column id -> declared column. Non-empty.
    pub fields: BTreeMap<String, CatalogueField>,
    /// Entry id -> {field id -> value}. Non-empty; every entry supplies exactly
    /// the declared fields, no more and no fewer.
    pub entries: BTreeMap<String, BTreeMap<String, Value>>,
    pub doc: Option<String>,
}

/// One shared choice of a catalogue entry (ADR-0057 §D3).
///
/// A binding **is** a declared closed facet whose values are its catalogue's
/// entry ids in id-ascending order. It shares the facet id space (a binding
/// named like a facet is `E_INGEST_DUPLICATE_FACET`), so environments bind it
/// in `choices` or `context_tags`, constraints reference it, `cfx options`
/// lists it, and the CCM carries its `exactly_one_of` cardinality — nothing
/// downstream special-cases it.
///
/// `default` and `derive` are mutually exclusive. A `derive` table is authoring
/// sugar: each `(source_value -> entry)` pair lowers to the root conjunct
/// `src != 'k' || binding == 'e'` (ADR-0057 §D4 — the lowering lands with
/// `requires`, in configflux-secb.5; this task carries the table verbatim
/// through the IR).
///
/// Data shape only. Catalogue existence, `default` membership, derive-source
/// declaration, and derive key/value membership are validated over the merged
/// [`crate::interface_summary::MergedSummary`] (`E_BINDING_INVALID`).
#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Binding {
    /// The catalogue whose entries are this binding's value domain.
    pub catalogue: String,
    /// The default entry; when present it must be one of the catalogue's
    /// entries, and `derive` must be absent.
    pub default: Option<String>,
    /// Authoring sugar: `{source_facet_or_binding: {source_value: entry_id}}`.
    /// Exactly one source in v1; the table may be partial (an uncovered source
    /// value implies nothing).
    pub derive: Option<BTreeMap<String, BTreeMap<String, String>>>,
    pub doc: Option<String>,
}

// ============================================================================
// 7. Artifacts
// ============================================================================

#[derive(Debug, Deserialize, Serialize, PartialEq, Clone)]
pub struct Artifact {
    pub name: String,
    pub version: Option<String>,
    pub hash: Option<String>,
    pub source: Option<String>,
    pub target: Option<String>,
    pub doc: Option<String>,
}
