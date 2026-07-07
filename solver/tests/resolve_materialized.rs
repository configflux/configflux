// SPDX-License-Identifier: BUSL-1.1
//
// `Session::resolve()` materialized-output + deterministic resolve_hash —
// configflux-i0ne (g3f.2 dual-path-parity blocker).
//
// Exercises the real `Session::resolve()` walk end-to-end over
// `OxiddBackend` on the same hand-crafted three-variable CCM fixture
// `apply_retract.rs` uses: `engine.v6` (var 0), `engine.v8` (var 1),
// `gearbox.auto` (var 2), base formula `(engine.v6 XOR engine.v8) ∧
// gearbox.auto`.
//
// Approach: ADAPTER (task option 2). The solver crate owns only the
// boolean selection model (the `{facet}.{value}` BDD variables), not the
// compiler's component/artifact catalog, so `Session::resolve()` produces
// the solver's authoritative resolved configuration — a canonical
// facet -> selected-option map — plus a deterministic `resolve_hash` over
// `(ccm_hash, bound_model_hash, canonical resolved_output)`. The compiler
// API supplies the component/artifact fields of the legacy ResolveResult
// in the g3f.2 dual-path wiring; that is out of this task's scope.
//
// Acceptance coverage:
//   (1) resolve() materializes a non-empty facet -> option map consistent
//       with `valid_options` (each facet's chosen option is the first
//       still-satisfiable value in symbol-table order);
//   (2) resolve_hash is deterministic: two independent loads of the same
//       artifact with the same applied selection produce the same hash;
//   (3) a different applied selection produces a different resolved_output
//       and a different resolve_hash;
//   (4) `apply` that narrows the configuration is reflected in resolve()'s
//       output (post-apply resolved_output pins the applied value);
//   (5) the empty-Ccm path stays satisfiable with an empty resolved_output
//       and a stable (deterministic) hash.
//
// Never imports `oxidd::*` (ADR-0003 §2/§3).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use solver::{Ccm, OxiddBackend, Session};

// Re-declared ADR-0005 §4 constants. Kept local so this test does not
// depend on `ccm_format`'s `pub(crate)` surface; drift between these and
// the crate internals surfaces here as a fixture that no longer parses —
// the same contract `apply_retract.rs` / `valid_options_bdd.rs` follow.
const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

/// Build a `ccm.bdd.bin` encoding `(engine.v6 XOR engine.v8) ∧
/// gearbox.auto` under var order [v6=0, v8=1, auto=2]. Post-order table:
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

fn tempdir_for(test_name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "configflux-solver-resolve-{}-{}",
        test_name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}

fn materialize(base: &Path) -> PathBuf {
    let bound = "aa".repeat(32);
    let bdd_bytes = build_xor_and_auto_bdd_bin();
    let symbols_bytes = build_symbols_json();
    fixture_v2::materialize_single_partition_ccm(base, &bdd_bytes, &symbols_bytes, &bound, 3, 6)
}

fn load_xor_and_auto_session(tempdir_label: &str) -> Session<OxiddBackend> {
    let base = tempdir_for(tempdir_label);
    let dir = materialize(&base);
    let ccm: Ccm = Session::<OxiddBackend>::load_ccm(&dir).expect("load fixture");
    Session::<OxiddBackend>::new(ccm).expect("session from fixture")
}

#[test]
fn resolve_materializes_canonical_selection_consistent_with_valid_options() {
    // Acceptance #1: resolve() must return a non-empty, materialized
    // facet -> option map and report the model satisfiable. Each chosen
    // option must equal the first still-satisfiable value `valid_options`
    // returns for that facet (the deterministic pick_one walk).
    let session = load_xor_and_auto_session("materialize");
    let result = session.resolve().expect("resolve must succeed on a SAT model");

    assert!(result.satisfiable, "fixture model is satisfiable");
    assert!(
        !result.resolved_output.is_empty(),
        "resolve() must materialize a non-empty resolved configuration, got {:?}",
        result.resolved_output,
    );

    // gearbox has a single forced option (auto) — resolve must pick it.
    assert_eq!(
        result.resolved_output.get("gearbox").map(String::as_str),
        Some("auto"),
        "gearbox must resolve to its only satisfiable option `auto`",
    );

    // engine has two options {v6, v8}; the canonical pick is the first in
    // symbol-table order (v6 at var 0). It must be one of the valid
    // options and must match valid_options' first surviving value.
    let engine_choice = result
        .resolved_output
        .get("engine")
        .expect("engine facet must appear in resolved_output")
        .clone();
    let engine_valid = session
        .valid_options("engine")
        .expect("valid_options(engine) must succeed")
        .options;
    assert!(
        engine_valid.contains(&engine_choice),
        "resolved engine option {engine_choice:?} must be a valid option of {engine_valid:?}",
    );
    assert_eq!(
        Some(&engine_choice),
        engine_valid.first(),
        "resolve() must pick the first still-satisfiable option in symbol-table order",
    );
}

#[test]
fn resolve_hash_is_deterministic_across_independent_loads() {
    // Acceptance #2: same artifact + same (empty) selection => identical
    // resolve_hash and identical resolved_output across two independent
    // loads. This is the determinism guarantee the dual-path parity test
    // (g3f.2) depends on.
    let a = load_xor_and_auto_session("determinism_a")
        .resolve()
        .expect("resolve a");
    let b = load_xor_and_auto_session("determinism_b")
        .resolve()
        .expect("resolve b");

    assert_eq!(
        a.resolved_output, b.resolved_output,
        "resolved_output must be identical across independent loads of the same artifact",
    );
    assert_eq!(
        a.resolve_hash, b.resolve_hash,
        "resolve_hash must be byte-identical across independent loads of the same artifact",
    );
    // A real resolve hash is never all-zero for a non-empty resolved model.
    assert_ne!(
        a.resolve_hash, [0u8; 32],
        "a materialized resolve over a non-empty model must produce a non-zero hash",
    );
}

#[test]
fn different_selection_changes_resolved_output_and_hash() {
    // Acceptance #3 + #4: applying a selection that narrows the model must
    // change both resolved_output and resolve_hash relative to the
    // baseline. Applying `engine.v8` forces the XOR the other way, so the
    // resolved engine option flips from the baseline `v6` pick to `v8`.
    let baseline = load_xor_and_auto_session("select_baseline")
        .resolve()
        .expect("baseline resolve");
    assert_eq!(
        baseline.resolved_output.get("engine").map(String::as_str),
        Some("v6"),
        "baseline canonical pick for engine is v6 (first in symbol-table order)",
    );

    let mut narrowed = load_xor_and_auto_session("select_narrowed");
    narrowed
        .apply("engine", "v8")
        .expect("apply(engine, v8) must succeed on the baseline model");
    let narrowed = narrowed.resolve().expect("narrowed resolve");

    assert_eq!(
        narrowed.resolved_output.get("engine").map(String::as_str),
        Some("v8"),
        "after apply(engine, v8) the resolved engine option must be v8",
    );
    assert_ne!(
        baseline.resolved_output, narrowed.resolved_output,
        "a different selection must produce a different resolved_output",
    );
    assert_ne!(
        baseline.resolve_hash, narrowed.resolve_hash,
        "a different selection must produce a different resolve_hash",
    );
}

#[test]
fn resolve_is_idempotent_for_the_same_state() {
    // resolve() is a pure query: calling it twice on the same session
    // state must return byte-identical output and hash.
    let session = load_xor_and_auto_session("idempotent");
    let first = session.resolve().expect("first resolve");
    let second = session.resolve().expect("second resolve");
    assert_eq!(first.resolved_output, second.resolved_output);
    assert_eq!(first.resolve_hash, second.resolve_hash);
    assert_eq!(first.satisfiable, second.satisfiable);
}

#[test]
fn empty_ccm_resolves_satisfiable_with_empty_output_and_stable_hash() {
    // Acceptance #5: the empty-Ccm path (no symbols) must resolve as
    // satisfiable with an empty resolved configuration. The hash is
    // computed over the (empty) canonical output and must be deterministic
    // across loads — preserving the round-trip-empty contract.
    let ccm = Session::<OxiddBackend>::load_ccm(Path::new("unused-empty-path")).expect("empty ccm");
    let session = Session::<OxiddBackend>::new(ccm).expect("empty session");
    let result = session.resolve().expect("empty resolve must succeed");

    assert!(result.satisfiable, "the empty model is trivially satisfiable");
    assert!(
        result.resolved_output.is_empty(),
        "the empty model has no facets to resolve",
    );

    // Determinism on the empty path too.
    let ccm2 = Session::<OxiddBackend>::load_ccm(Path::new("unused-empty-path")).expect("empty ccm");
    let session2 = Session::<OxiddBackend>::new(ccm2).expect("empty session");
    let result2 = session2.resolve().expect("empty resolve must succeed");
    assert_eq!(
        result.resolve_hash, result2.resolve_hash,
        "empty-Ccm resolve_hash must be deterministic across loads",
    );
}
