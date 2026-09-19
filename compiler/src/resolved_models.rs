// SPDX-License-Identifier: BUSL-1.1

use crate::schema::{Lifecycle, Limits, Role, SafetyLevel, Value};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// The "100% Model".
/// Represents a Fully Resolved configuration for a specific target.
#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct ResolvedConfig {
    pub package: String,
    pub version: String,
    pub components: HashMap<String, ResolvedComponent>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct ResolvedComponent {
    pub r#type: String,
    /// What this component's requirements resolved to, keyed by slot
    /// (ADR-0057 §D7). The whole point of the block: a service reads its OWN
    /// configuration and never has to know which catalogue component holds the
    /// shared table, or what that table is called.
    ///
    /// `skip_serializing_if` is load-bearing. `resolved_output` is the largest
    /// member of the `resolve_hash` pre-image, so an always-present
    /// `"requires": {}` would rotate the `resolve_hash` of every scope in
    /// existence, including scopes that will never carry a requirement.
    /// Omitting the empty map is what makes a requirement-free snapshot's bytes
    /// move by the schema-version literal alone.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub requires: BTreeMap<String, ResolvedRequirement>,
    pub params: HashMap<String, ResolvedParameter>,
}

/// One requirement, resolved: which binding the slot named, which catalogue
/// entry that binding took in this deployment, and that entry's values
/// (ADR-0057 §D7).
///
/// `fields` is a `BTreeMap` for the same reason the authored catalogue's is:
/// the resolved snapshot is hashed, so field order has to be a property of the
/// data rather than of whichever map the producer happened to use. The
/// canonicalizer sorts object keys anyway; the `BTreeMap` makes the guarantee
/// hold in the Rust value too, so a consumer that iterates gets the same order
/// the bytes carry.
///
/// The entry is copied in rather than referenced. A snapshot is delivered to a
/// device and read there with no model at hand, so a pointer back into a
/// catalogue would be a dangling one.
#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct ResolvedRequirement {
    /// The binding the requirement named (ADR-0057 §D4).
    pub binding: String,
    /// The catalogue entry that binding resolved to in this deployment.
    pub entry: String,
    /// That entry's values, field id -> value, complete and exact: a catalogue
    /// entry supplies every declared field and nothing else
    /// (`link_verify::validate_catalogues`).
    pub fields: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct ResolvedParameter {
    // Value and Type are mandatory in the resolved model
    pub value: Value,
    pub r#type: String,

    /// The facet this parameter is the declared runtime handle for
    /// (ADR-0064 D4), when the model declared one. `value` above is then that
    /// facet's effective value in this deployment.
    ///
    /// This is the channel the binding reaches the runtime on: a resolved
    /// parameter travels inside `resolved_output`, which is on the runtime open
    /// contract AND inside the `resolve_hash` pre-image, so a tampered binding
    /// fails the open with `E_RUNTIME_HASH_MISMATCH` and needs no validation of
    /// its own.
    ///
    /// `skip_serializing_if` keeps an UNBOUND parameter byte-identical to what
    /// it was before the field existed, which is what leaves every golden, every
    /// byte-stability baseline and every `resolve_hash` of a model without
    /// bindings untouched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facet: Option<String>,

    pub unit: Option<String>,

    // Metadata (Defaults applied if missing in schema)
    pub safety: SafetyLevel,
    pub lifecycle: Lifecycle,
    pub access: Role,
    pub req_id: Option<String>,
    pub doc: Option<String>,

    pub limits: Option<Limits>,
}
