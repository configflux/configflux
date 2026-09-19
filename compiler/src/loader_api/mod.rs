// SPDX-License-Identifier: BUSL-1.1

// configflux-ccs.7: selection-constraint resolution now uses the typed
// `ConditionExpr` path. The former string-scanning helpers
// (`parse_condition_conjunction` / `scan_condition_predicates`) were removed in
// configflux-uiyo once the last matrix-implication user (`link_verify`) moved
// onto the typed evaluator; only the typed helpers below are imported here.
use crate::conditions::{
    for_each_eq_predicate, is_contradicted, is_pure_conjunction, mentions_eq, not_contradicted,
    parse_condition_expr, ConditionExpr, FacetWorld,
};
use crate::ir::{
    self, CMP_CANONICALIZATION_VERSION, CMP_HASH_ALGO, CMP_MANIFEST_SCHEMA_VERSION,
    IR_FORMAT_VERSION,
};
use crate::product_api::{
    Diagnostic, DiagnosticSeverity, DiagnosticsReport, OperationStatus, PRODUCT_SCHEMA_VERSION,
};
use crate::resolver::{self, ResolutionContext};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

include!("contracts.rs");
include!("operations.rs");
// ADR-0054 §5.4 (configflux-p571.8): maps an unsat-core clause back to the
// authored constraint that forbids it, using the top-level manifest roster.
// Pure data in, pure data out — no solver type crosses the ADR-0003 §2 line.
include!("unsat_attribution.rs");

#[cfg(test)]
mod tests;
