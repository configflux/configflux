// SPDX-License-Identifier: BUSL-1.1
//
// `valid_options` over BDD projection — configflux-8dm.3.
//
// Hand-crafted three-variable CCM fixture exercising
// `Session::valid_options` end-to-end over `OxiddBackend`. Three BDD
// variables under the ADR-0005 §3 `(tag, value)` naming convention:
// `engine.v6` (var 0), `engine.v8` (var 1), `gearbox.auto` (var 2).
// Base formula: `(engine.v6 XOR engine.v8) ∧ gearbox.auto`; a second
// "v8-only" fixture covers the empty-projection and unknown-facet
// acceptance paths. Never imports `oxidd::*` (ADR-0003 §2/§3).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use solver::{Ccm, Error, OxiddBackend, Session};

// Re-declared ADR-0005 §4 constants. `ccm_format` keeps the real
// definitions as `pub(crate)`; drift between these and the crate
// internals surfaces here as a fixture that no longer parses.
const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

/// Build a `ccm.bdd.bin` encoding
/// `(engine.v6 XOR engine.v8) ∧ gearbox.auto` under var order
/// [v6=0, v8=1, auto=2] in post-order:
///
///   0: ⊥, 1: ⊤,
///   2: auto   { var=2, low=⊥, high=⊤ },
///   3: v8-pos { var=1, low=2, high=⊥ }   (v6=1 branch: v8 must be 0, auto must be 1)
///   4: v8-neg { var=1, low=⊥, high=2 }   (v6=0 branch: v8 must be 1, auto must be 1)
///   5: v6     { var=0, low=4, high=3 }
///
/// Root = 5. Reducedness: nodes 3 and 4 differ in (low, high) tuples so
/// the §4 canonical key is distinct; no node has low == high.
fn build_xor_and_auto_bdd_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    // Header.
    bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
    bytes.push(CCM_BDD_BIN_VERSION);
    bytes.extend_from_slice(&[0u8; 3]); // reserved
    bytes.extend_from_slice(&3u32.to_le_bytes()); // var_count
    bytes.extend_from_slice(&6u32.to_le_bytes()); // node_count
    bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count
    bytes.extend_from_slice(&5u32.to_le_bytes()); // root[0] = node 5

    // Node 0: terminal FALSE.
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    // Node 1: terminal TRUE.
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    // Node 2: auto-node { var=2, low=⊥, high=⊤ }.
    push_node(&mut bytes, 2, 0, 1);
    // Node 3: v8-node-pos { var=1, low=2 (auto), high=⊥ }.
    push_node(&mut bytes, 1, 2, 0);
    // Node 4: v8-node-neg { var=1, low=⊥, high=2 (auto) }.
    push_node(&mut bytes, 1, 0, 2);
    // Node 5: v6-node { var=0, low=4 (v8-neg), high=3 (v8-pos) }.
    push_node(&mut bytes, 0, 4, 3);

    bytes
}

fn push_node(out: &mut Vec<u8>, var: u32, low: u32, high: u32) {
    out.extend_from_slice(&var.to_le_bytes());
    out.extend_from_slice(&low.to_le_bytes());
    out.extend_from_slice(&high.to_le_bytes());
    out.push(0u8);
    out.extend_from_slice(&[0u8; 3]);
}

/// Build a matching `ccm.symbols.json` for the fixture above.
fn build_symbols_json() -> Vec<u8> {
    let mut facet_to_var = BTreeMap::new();
    facet_to_var.insert("engine.v6".to_string(), 0u32);
    facet_to_var.insert("engine.v8".to_string(), 1u32);
    facet_to_var.insert("gearbox.auto".to_string(), 2u32);
    let sym = SymbolsOut {
        schema_version: 2,
        variable_order: vec![
            "engine.v6".to_string(),
            "engine.v8".to_string(),
            "gearbox.auto".to_string(),
        ],
        facet_to_var,
        var_to_label: vec![
            "engine=v6".to_string(),
            "engine=v8".to_string(),
            "gearbox=auto".to_string(),
        ],
    };
    let mut v = serde_json::to_vec(&sym).expect("symbols serialize");
    v.push(b'\n');
    v
}

#[derive(Serialize)]
struct SymbolsOut {
    schema_version: u32,
    variable_order: Vec<String>,
    facet_to_var: BTreeMap<String, u32>,
    var_to_label: Vec<String>,
}

// v2 multi-part CCM fixture builders are shared across the solver
// integration tests; see `fixture_v2.rs`.
#[path = "fixture_v2.rs"]
mod fixture_v2;

/// Lay the v2 multi-part CCM under `base/ccm/` and return the directory.
fn materialize_ccm_dir(
    base: &Path,
    bdd_bytes: &[u8],
    symbols_bytes: &[u8],
    bound_model_hash_hex: &str,
    var_count: u32,
    node_count: u64,
) -> PathBuf {
    fixture_v2::materialize_single_partition_ccm(
        base,
        bdd_bytes,
        symbols_bytes,
        bound_model_hash_hex,
        var_count,
        node_count,
    )
}

fn tempdir_for(test_name: &str) -> PathBuf {
    fixture_v2::unique_temp_dir("configflux-solver-valid-options", test_name)
}

fn load_xor_and_auto_session(tempdir_label: &str) -> Session<OxiddBackend> {
    let base = tempdir_for(tempdir_label);
    let bound = "aa".repeat(32);
    let bdd_bytes = build_xor_and_auto_bdd_bin();
    let symbols_bytes = build_symbols_json();
    let dir = materialize_ccm_dir(&base, &bdd_bytes, &symbols_bytes, &bound, 3, 6);
    let ccm: Ccm = Session::<OxiddBackend>::load_ccm(&dir).expect("load fixture");
    Session::<OxiddBackend>::new(ccm).expect("session from fixture")
}

#[test]
fn valid_options_enumerates_both_engine_values_under_xor() {
    // Acceptance #1: the XOR-AND-auto fixture has two valid engine
    // assignments (v6 and v8 are each individually satisfiable). The
    // returned option set must be exactly {v6, v8} — no ordering
    // requirement, but the set identity must match.
    let session = load_xor_and_auto_session("xor_engine");
    let options = session
        .valid_options("engine")
        .expect("valid_options must succeed on known facet");
    let mut got: Vec<String> = options.options.clone();
    got.sort();
    assert_eq!(got, vec!["v6".to_string(), "v8".to_string()]);
    assert_eq!(options.count, 2, "count must match options.len()");
}

#[test]
fn valid_options_returns_single_forced_value_for_gearbox() {
    // Acceptance #1 (forced-option variant): gearbox.auto is forced by
    // the constraint, so its sole option value is still the only
    // satisfiable one.
    let session = load_xor_and_auto_session("xor_gearbox");
    let options = session
        .valid_options("gearbox")
        .expect("valid_options must succeed on known facet");
    assert_eq!(options.options, vec!["auto".to_string()]);
    assert_eq!(options.count, 1);
}

#[test]
fn valid_options_unknown_facet_returns_typed_error() {
    // Acceptance #2: a facet name that is not a prefix of any symbol
    // must return a typed `Error::UnknownFacet`, not a silent empty
    // result (which would be indistinguishable from "all options
    // pruned") and not a panic.
    let session = load_xor_and_auto_session("xor_unknown");
    let err = session
        .valid_options("transmission")
        .expect_err("unknown facet must return Err");
    assert!(
        matches!(&err, Error::UnknownFacet(name) if name == "transmission"),
        "expected Error::UnknownFacet(\"transmission\"), got {err:?}"
    );
}

#[test]
fn valid_options_on_empty_ccm_is_empty_not_error() {
    // Acceptance #3: `valid_options` on an empty CCM (no symbols, no
    // BDD) yields an empty option list with `count == 0`, not an error.
    // This preserves the M0 round-trip-empty contract where callers
    // check `opts.count == 0` on a just-loaded empty session.
    let session: Session<OxiddBackend> = Session::<OxiddBackend>::new(Ccm::empty())
        .expect("empty-Ccm session must construct");
    let options = session
        .valid_options("engine")
        .expect("valid_options on empty Ccm must return Ok");
    assert_eq!(options.count, 0);
    assert!(options.options.is_empty());
}

#[test]
fn valid_options_enumerates_only_sat_values_in_unsat_branch() {
    // Acceptance #3 (empty-projection path): use a fixture whose formula
    // `(¬engine.v6) ∧ engine.v8 ∧ gearbox.auto` explicitly forbids v6,
    // so `valid_options("engine")` must be `{v8}` — not `{v6, v8}` and
    // not an error. Post-order:
    //   0: ⊥, 1: ⊤,
    //   2: auto  { var=2, low=⊥, high=⊤ },
    //   3: v8    { var=1, low=⊥, high=2 },   (v8 must be 1; then auto must be 1)
    //   4: v6    { var=0, low=3, high=⊥ }.   (v6 must be 0; else ⊥)
    // Root = 4.
    let base = tempdir_for("v8_only");
    let bound = "bb".repeat(32);
    let mut bdd_bytes = Vec::new();
    bdd_bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
    bdd_bytes.push(CCM_BDD_BIN_VERSION);
    bdd_bytes.extend_from_slice(&[0u8; 3]);
    bdd_bytes.extend_from_slice(&3u32.to_le_bytes()); // var_count
    bdd_bytes.extend_from_slice(&5u32.to_le_bytes()); // node_count
    bdd_bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count
    bdd_bytes.extend_from_slice(&4u32.to_le_bytes()); // root = 4
    push_node(&mut bdd_bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    push_node(&mut bdd_bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    push_node(&mut bdd_bytes, 2, 0, 1); // auto
    push_node(&mut bdd_bytes, 1, 0, 2); // v8 (low=⊥, high=auto)
    push_node(&mut bdd_bytes, 0, 3, 0); // v6 (low=v8-node, high=⊥)

    let symbols_bytes = build_symbols_json();
    let dir = materialize_ccm_dir(&base, &bdd_bytes, &symbols_bytes, &bound, 3, 5);
    let ccm: Ccm = Session::<OxiddBackend>::load_ccm(&dir).expect("load v8-only fixture");
    let session = Session::<OxiddBackend>::new(ccm).expect("session from v8-only fixture");

    let options = session
        .valid_options("engine")
        .expect("valid_options must succeed even when one value is unsat");
    assert_eq!(options.options, vec!["v8".to_string()]);
    assert_eq!(options.count, 1);
}
