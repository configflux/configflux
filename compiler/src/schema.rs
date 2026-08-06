// SPDX-License-Identifier: BUSL-1.1

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    #[serde(default)]
    pub params: HashMap<String, Parameter>,
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
