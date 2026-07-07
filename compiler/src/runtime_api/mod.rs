// SPDX-License-Identifier: BUSL-1.1

use crate::product_api::{
    Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};

include!("contracts.rs");
include!("operations.rs");

#[cfg(test)]
mod tests;
#[cfg(test)]
mod authority_tests;
