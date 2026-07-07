// SPDX-License-Identifier: BUSL-1.1

use crate::schema::{Lifecycle, Limits, Role, SafetyLevel, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    pub params: HashMap<String, ResolvedParameter>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Serialize)]
pub struct ResolvedParameter {
    // Value and Type are mandatory in the resolved model
    pub value: Value,
    pub r#type: String,

    pub unit: Option<String>,

    // Metadata (Defaults applied if missing in schema)
    pub safety: SafetyLevel,
    pub lifecycle: Lifecycle,
    pub access: Role,
    pub req_id: Option<String>,
    pub doc: Option<String>,

    pub limits: Option<Limits>,
}
