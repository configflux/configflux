// SPDX-License-Identifier: BUSL-1.1
//
// `apply` / `retract` round-trip — configflux-8dm.4.
//
// Hand-crafted three-variable CCM fixture exercising
// `Session::apply` and `Session::retract` end-to-end over `OxiddBackend`.
// Three BDD variables under the ADR-0005 §3 `(tag, value)` naming
// convention: `engine.v6` (var 0), `engine.v8` (var 1), `gearbox.auto`
// (var 2). Base formula: `(engine.v6 XOR engine.v8) ∧ gearbox.auto`.
//
// Acceptance coverage:
//   (1) forward `apply("engine","v6")` narrows `valid_options("engine")`
//       from `{v6, v8}` down to `{v6}`;
//   (2) `retract("engine")` restores the pre-apply `valid_options`;
//   (3) conflicting `apply("engine","v8")` after `apply("engine","v6")`
//       returns `Error::Conflict` without corrupting session state, and
//       the prior `valid_options` observation survives the failed call.
//
// Never imports `oxidd::*` (ADR-0003 §2/§3).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use solver::{Ccm, Error, OxiddBackend, Session};

// Re-declared ADR-0005 §4 constants. Kept local so this test does not
// depend on `ccm_format`'s `pub(crate)` surface; drift between these
// and the crate internals surfaces here as a fixture that no longer
// parses — the same contract `valid_options_bdd.rs` follows.
const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

/// Build a `ccm.bdd.bin` encoding `(engine.v6 XOR engine.v8) ∧
/// gearbox.auto` under var order [v6=0, v8=1, auto=2]. Matches the
/// post-order table in `valid_options_bdd.rs`:
///
///   0: ⊥, 1: ⊤,
///   2: auto   { var=2, low=⊥, high=⊤ },
///   3: v8-pos { var=1, low=2, high=⊥ },
///   4: v8-neg { var=1, low=⊥, high=2 },
///   5: v6     { var=0, low=4, high=3 }.
///
/// Root = 5.
fn build_xor_and_auto_bdd_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(CCM_BDD_BIN_MAGIC);
    bytes.push(CCM_BDD_BIN_VERSION);
    bytes.extend_from_slice(&[0u8; 3]); // reserved
    bytes.extend_from_slice(&3u32.to_le_bytes()); // var_count
    bytes.extend_from_slice(&6u32.to_le_bytes()); // node_count
    bytes.extend_from_slice(&1u32.to_le_bytes()); // root_count
    bytes.extend_from_slice(&5u32.to_le_bytes()); // root[0] = node 5

    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    push_node(&mut bytes, 2, 0, 1); // auto
    push_node(&mut bytes, 1, 2, 0); // v8-pos
    push_node(&mut bytes, 1, 0, 2); // v8-neg
    push_node(&mut bytes, 0, 4, 3); // v6

    bytes
}

fn push_node(out: &mut Vec<u8>, var: u32, low: u32, high: u32) {
    out.extend_from_slice(&var.to_le_bytes());
    out.extend_from_slice(&low.to_le_bytes());
    out.extend_from_slice(&high.to_le_bytes());
    out.push(0u8);
    out.extend_from_slice(&[0u8; 3]);
}

/// Build a matching `ccm.symbols.json` for the fixture.
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
// integration tests; see `fixture_v2.rs` for the canonical hash recipe.
#[path = "fixture_v2.rs"]
mod fixture_v2;

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
    let base = std::env::temp_dir().join(format!(
        "configflux-solver-apply-retract-{}-{}",
        test_name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
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

fn sorted_engine_options(session: &Session<OxiddBackend>) -> Vec<String> {
    let opts = session
        .valid_options("engine")
        .expect("valid_options on engine must succeed");
    let mut v = opts.options.clone();
    v.sort();
    v
}

#[test]
fn apply_narrows_valid_options_then_retract_restores() {
    // Acceptance #1 + #2: forward apply narrows the option set, and
    // retract pops the undo stack so subsequent valid_options calls
    // return the pre-apply set.
    let mut session = load_xor_and_auto_session("narrow_restore");

    // Baseline: both engine values are valid.
    assert_eq!(
        sorted_engine_options(&session),
        vec!["v6".to_string(), "v8".to_string()],
        "baseline engine options must be {{v6, v8}}"
    );

    // Apply v6 — conjoins the current formula with `engine.v6 = 1`.
    // Under the XOR constraint this forces `engine.v8 = 0`, so the
    // remaining option set for `engine` is {v6}.
    session
        .apply("engine", "v6")
        .expect("apply(engine, v6) must succeed");
    assert_eq!(
        sorted_engine_options(&session),
        vec!["v6".to_string()],
        "after apply(engine, v6) only v6 is still a valid option"
    );

    // Retract pops the undo stack and restores the pre-apply formula.
    session.retract("engine").expect("retract must succeed");
    assert_eq!(
        sorted_engine_options(&session),
        vec!["v6".to_string(), "v8".to_string()],
        "retract must restore pre-apply options",
    );
}

#[test]
fn conflicting_apply_returns_conflict_error_without_corrupting_state() {
    // Acceptance #3: after `apply(engine, v6)` the session has `v6=1`
    // conjoined; a follow-up `apply(engine, v8)` would force `v8=1`,
    // which is unsat under the `v6 XOR v8` base formula. The call
    // must return `Error::Conflict` and MUST NOT mutate session
    // state — `valid_options` afterwards still shows the narrowed
    // {v6}-only set and a `retract` still restores the baseline.
    let mut session = load_xor_and_auto_session("conflict_state");

    session
        .apply("engine", "v6")
        .expect("first apply must succeed");
    let narrowed = sorted_engine_options(&session);
    assert_eq!(
        narrowed,
        vec!["v6".to_string()],
        "sanity: narrowed set is {{v6}} before the conflicting apply",
    );

    // Conflicting apply: v8 is incompatible with v6 under XOR.
    let err = session
        .apply("engine", "v8")
        .expect_err("apply(engine, v8) after apply(engine, v6) must conflict");
    assert!(
        matches!(&err, Error::Conflict { facet, value } if facet == "engine" && value == "v8"),
        "expected Error::Conflict {{ facet: engine, value: v8 }}, got {err:?}",
    );

    // State preservation: `current` must still reflect the {v6}-only
    // narrowing — the failed apply must not have pushed anything onto
    // the undo stack nor swapped `current`.
    assert_eq!(
        sorted_engine_options(&session),
        narrowed,
        "state must not change when apply returns Conflict",
    );

    // And the undo stack must still contain exactly the one prior
    // `apply(engine, v6)` entry — a single `retract` restores the
    // baseline, a second `retract` is a no-op.
    session
        .retract("engine")
        .expect("retract must succeed after a conflict");
    assert_eq!(
        sorted_engine_options(&session),
        vec!["v6".to_string(), "v8".to_string()],
        "retract must restore the baseline set",
    );
    session
        .retract("engine")
        .expect("retract on empty undo stack is a no-op, not an error");
}

#[test]
fn unknown_option_returns_typed_error() {
    // `apply(facet, value)` must map an unknown `(facet, value)` pair
    // — i.e. a symbol that is not in `ccm.symbols.json` — to a typed
    // `Error::UnknownOption`, not `Conflict`. Keeps the error model
    // parallel to `valid_options`'s `UnknownFacet` vs. empty-Vec
    // distinction (configflux-8dm.3): "I don't know this" and
    // "this is unsat" are different outcomes.
    let mut session = load_xor_and_auto_session("unknown_option");
    let err = session
        .apply("engine", "v12")
        .expect_err("apply on an unknown value must return Err");
    assert!(
        matches!(&err, Error::UnknownOption { facet, value } if facet == "engine" && value == "v12"),
        "expected Error::UnknownOption {{ facet: engine, value: v12 }}, got {err:?}",
    );
    // Failed lookup must not have mutated state.
    assert_eq!(
        sorted_engine_options(&session),
        vec!["v6".to_string(), "v8".to_string()],
        "unknown-option apply must not mutate state",
    );
}
