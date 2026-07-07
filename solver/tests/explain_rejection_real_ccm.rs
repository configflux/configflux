// SPDX-License-Identifier: BUSL-1.1
//
// `Session::explain_rejection` over a *real-serializer* `.ccm` — configflux-autp.
//
// REGRESSION TEST for the index-based-terminal fault. The existing
// `explain_rejection_mus.rs` (configflux-kv5d) suite exercises ONLY
// hand-rolled BDD fixtures whose non-terminal nodes reference the ⊥/⊤
// terminals using the SENTINEL constants (`TERMINAL_FALSE=0xFFFFFFFF /
// TERMINAL_TRUE=0xFFFFFFFE`). The real oxidd/cudd compiler serializer
// (`compiler/src/ccm_emitter/bdd.rs::child_id`) instead references the
// terminals BY TABLE INDEX — `0 = ⊥`, `1 = ⊤` (ADR-0005 §4). Sentinels
// only ever appear in the two terminal nodes' own backpointer fields and
// as a *constant* root.
//
// `Session::explain_rejection`'s BDD falsifying-path walker
// (`solver/src/explain.rs::walk_node` / `enumerate_falsifying_clauses`) was
// the one consumer that rejected the index form: following a `low_id` /
// `high_id` of `0` or `1` landed on a terminal-tagged node and tripped the
// `"explain: terminal-tagged node referenced by index, not sentinel"`
// invariant — faulting EVERY real-model explain with
// `Err(Backend(Invariant(..)))`, including genuinely-valid options. The
// loader (`backend_oxidd::lookup_child`) and the format validator
// (`ccm_format.rs`) already accept both forms; this regression pins that the
// explain path now does too.
//
// To reproduce the real serializer faithfully WITHOUT depending on the
// compiler crate (ADR-0003 §2: no compiler type crosses into `solver/`),
// the BDD here is hand-assembled byte-for-byte using the SAME index-based
// terminal-child convention `child_id` emits: every non-terminal node that
// points at ⊥ / ⊤ uses index `0` / `1`, never a sentinel. This is the exact
// on-disk shape `load_ccm_real_fixture.rs` already round-trips through
// `Session::load_ccm`; the gap this test closes is that `explain_rejection`
// was never driven over that shape. The end-to-end `compile_model -> .ccm ->
// explain_rejection` path is covered downstream by the runtime / interpreter
// E2E fixtures (configflux-3b5y / configflux-whyt), which activate once this
// fix lands.
//
// Mirrors the `s_solver` runtime fixture's semantics (a `cooling_brand`
// facet whose feasible space requires `cooling_brand.hydra` and forbids
// `cooling_brand.aeroflux`): explaining the forbidden option returns a
// labeled core; explaining the required option returns Ok with no core.
//
// Never imports `oxidd::*` or `batsat::*` (ADR-0003 §2/§3, ADR-0004 §4).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use solver::{Error, OxiddBackend, Session};

// Re-declared ADR-0005 §4 constants, local so this test does not depend on
// `ccm_format`'s `pub(crate)` surface (the same contract every other fixture
// test follows). NOTE: the terminal *children* below are written as plain
// table indices `0` / `1`, NOT these sentinels — that is the whole point.
const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

#[path = "fixture_v2.rs"]
mod fixture_v2;

// ---------------------------------------------------------------------------
// BDD + symbols builders (real-serializer index-based terminal convention)
// ---------------------------------------------------------------------------

fn push_node(out: &mut Vec<u8>, var: u32, low: u32, high: u32) {
    out.extend_from_slice(&var.to_le_bytes());
    out.extend_from_slice(&low.to_le_bytes());
    out.extend_from_slice(&high.to_le_bytes());
    out.push(0u8);
    out.extend_from_slice(&[0u8; 3]);
}

fn bdd_header(out: &mut Vec<u8>, var_count: u32, node_count: u32, root: u32) {
    out.extend_from_slice(CCM_BDD_BIN_MAGIC);
    out.push(CCM_BDD_BIN_VERSION);
    out.extend_from_slice(&[0u8; 3]); // reserved
    out.extend_from_slice(&var_count.to_le_bytes());
    out.extend_from_slice(&node_count.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // root_count
    out.extend_from_slice(&root.to_le_bytes()); // root[0]
}

/// `ccm.bdd.bin` for the feasible space `cooling_brand.hydra ∧
/// ¬cooling_brand.aeroflux` under var order
/// `[cooling_brand.aeroflux = 0, cooling_brand.hydra = 1]`.
///
///   0: ⊥, 1: ⊤,
///   2: hydra-node    { var=1, low=0,  high=1 }   (¬aeroflux cofactor)
///   3: aeroflux-node { var=0, low=2,  high=0 }   (root)
///
/// CRITICAL: nodes 2 and 3 reference the ⊥ / ⊤ terminals BY INDEX
/// (`0` / `1`) — the convention the real oxidd/cudd serializer emits
/// (`compiler/src/ccm_emitter/bdd.rs::child_id`), NOT the sentinel
/// constants the kv5d fixtures use. The two terminal nodes (0, 1) still
/// carry their sentinel backpointers, matching the real emitter.
///
/// Root = 3. Falsifying paths (root -> ⊥):
///   - aeroflux=1            -> clause `(¬cooling_brand.aeroflux)`
///   - aeroflux=0, hydra=0   -> clause `(aeroflux ∨ cooling_brand.hydra)`
///
/// So the option `cooling_brand.aeroflux` is forbidden (the unit rule
/// `(¬aeroflux)`), and `cooling_brand.hydra` is the satisfiable selection.
fn build_require_hydra_forbid_aeroflux_bdd_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    bdd_header(&mut bytes, 2, 4, 3);
    // index 0: ⊥ terminal (sentinel backpointers — real-emitter shape).
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    // index 1: ⊤ terminal.
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    // index 2: hydra (var 1). low -> ⊥ BY INDEX 0; high -> ⊤ BY INDEX 1.
    push_node(&mut bytes, 1, 0, 1);
    // index 3: aeroflux (var 0), root. low -> hydra-node (2); high -> ⊥ BY INDEX 0.
    push_node(&mut bytes, 0, 2, 0);
    bytes
}

#[derive(Serialize)]
struct SymbolsOut {
    schema_version: u32,
    variable_order: Vec<String>,
    facet_to_var: BTreeMap<String, u32>,
    var_to_label: Vec<String>,
}

/// `ccm.symbols.json` for the two-option `cooling_brand` fixture, var order
/// `[cooling_brand.aeroflux = 0, cooling_brand.hydra = 1]`.
fn build_cooling_brand_symbols_json() -> Vec<u8> {
    let mut facet_to_var = BTreeMap::new();
    facet_to_var.insert("cooling_brand.aeroflux".to_string(), 0u32);
    facet_to_var.insert("cooling_brand.hydra".to_string(), 1u32);
    let sym = SymbolsOut {
        schema_version: 2,
        variable_order: vec![
            "cooling_brand.aeroflux".to_string(),
            "cooling_brand.hydra".to_string(),
        ],
        facet_to_var,
        var_to_label: vec![
            "cooling_brand=aeroflux".to_string(),
            "cooling_brand=hydra".to_string(),
        ],
    };
    let mut v = serde_json::to_vec(&sym).expect("symbols serialize");
    v.push(b'\n');
    v
}

// ---------------------------------------------------------------------------
// Fixture materialization
// ---------------------------------------------------------------------------

fn tempdir_for(test_name: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "configflux-solver-explain-real-ccm-{}-{}",
        test_name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}

fn load_cooling_brand_session(label: &str) -> Session<OxiddBackend> {
    let base = tempdir_for(label);
    let bound = "cc".repeat(32);
    let bdd_bytes = build_require_hydra_forbid_aeroflux_bdd_bin();
    let symbols_bytes = build_cooling_brand_symbols_json();
    let dir: PathBuf = fixture_v2::materialize_single_partition_ccm(
        &base,
        &bdd_bytes,
        &symbols_bytes,
        &bound,
        2,
        4,
    );
    let ccm =
        Session::<OxiddBackend>::load_ccm(&dir).expect("load real-serializer cooling_brand fixture");
    Session::<OxiddBackend>::new(ccm).expect("session from cooling_brand fixture")
}

/// A label is a non-empty `String` that is not a bare integer — the
/// `explain_rejection` contract forbids raw BDD/batsat indices in any
/// returned value (ADR-0031 D3/D4).
fn assert_is_label(s: &str, context: &str) {
    assert!(!s.is_empty(), "{context}: label must be non-empty");
    assert!(
        s.parse::<i64>().is_err(),
        "{context}: '{s}' is a bare integer — a raw variable index leaked into the labeled core",
    );
    assert!(
        s.chars().any(|c| c.is_ascii_alphabetic()),
        "{context}: '{s}' has no alphabetic character — not a labeled name",
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn explain_rejection_over_index_based_terminal_ccm_labels_a_genuine_conflict() {
    // The regression: before the fix this returned
    // Err(Backend(Invariant("explain: terminal-tagged node referenced by
    // index, not sentinel"))) because the root's `high` and the hydra-node's
    // `low` reference ⊥ by index 0. The forbidden option `cooling_brand.aeroflux`
    // must now produce a labeled core, not a fault.
    let session = load_cooling_brand_session("forbidden_aeroflux");

    let explanation = session
        .explain_rejection("cooling_brand", "aeroflux")
        .expect(
            "explain_rejection must NOT fault on a real-serializer (index-based-terminal) .ccm; \
             a forbidden option must return Ok with a labeled core",
        );

    assert!(
        explanation.would_reject,
        "cooling_brand.aeroflux is forbidden by the feasible space (¬aeroflux) — must be rejected"
    );
    let core = explanation
        .core
        .as_ref()
        .expect("a rejected option must carry a labeled core");

    // The rejected atom is the candidate, labeled — never a raw index.
    assert_eq!(core.rejected.facet, "cooling_brand");
    assert_eq!(core.rejected.value, "aeroflux");
    assert_is_label(&core.rejected.facet, "core.rejected.facet");
    assert_is_label(&core.rejected.value, "core.rejected.value");
    assert!(core.minimal, "the core must be reported minimal after shrinking");

    // At least one labeled conflicting constraint, every atom a label.
    assert!(
        !core.conflicting_constraints.is_empty(),
        "a rejection must name at least one conflicting constraint"
    );
    let mut names_aeroflux = false;
    for (i, c) in core.conflicting_constraints.iter().enumerate() {
        for (j, atom) in c.atoms.iter().enumerate() {
            assert_is_label(&atom.facet, &format!("constraint[{i}].atoms[{j}].facet"));
            assert_is_label(&atom.value, &format!("constraint[{i}].atoms[{j}].value"));
            if atom.facet == "cooling_brand" && atom.value == "aeroflux" {
                names_aeroflux = true;
            }
        }
    }
    assert!(
        names_aeroflux,
        "the forbidding rule (¬cooling_brand.aeroflux) must name cooling_brand.aeroflux; got {:?}",
        core.conflicting_constraints,
    );
}

#[test]
fn explain_rejection_over_index_based_terminal_ccm_accepts_a_valid_option() {
    // The complementary half: a genuinely-valid option must return Ok with
    // would_reject:false and NO core — and, before the fix, this ALSO faulted
    // with the sentinel invariant (the walk hits the index-0/1 terminals for
    // every option). `cooling_brand.hydra` is the required/satisfiable option.
    let session = load_cooling_brand_session("valid_hydra");

    let explanation = session
        .explain_rejection("cooling_brand", "hydra")
        .expect(
            "explain_rejection on a valid option over a real-serializer .ccm must return Ok, \
             not fault on the index-based terminal references",
        );

    assert!(
        !explanation.would_reject,
        "cooling_brand.hydra is the required/satisfiable option — must not be rejected"
    );
    assert!(
        explanation.core.is_none(),
        "a genuinely valid option must carry no core, got {:?}",
        explanation.core,
    );
}

#[test]
fn unknown_option_is_a_typed_error_not_a_fault_on_real_ccm() {
    // Division-of-labor (ADR-0030 D5) still holds over the real-serializer
    // shape: an unknown option under the known facet is a typed UnknownOption,
    // distinct from the terminal-reference fault this issue fixed.
    let session = load_cooling_brand_session("unknown_option");
    let unknown = session.explain_rejection("cooling_brand", "glacier");
    assert!(
        matches!(&unknown, Err(Error::UnknownOption { facet, value })
            if facet == "cooling_brand" && value == "glacier"),
        "unknown option under a known facet must be UnknownOption, got {unknown:?}",
    );
}
