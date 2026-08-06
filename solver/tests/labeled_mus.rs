// SPDX-License-Identifier: BUSL-1.1
//
// Labeled-MUS exit-criterion invariant — configflux-vwhj (the configflux-osp
// M4 epic's HARD EXIT CRITERION made executable). The contract under test:
// `Session::explain_rejection` returns a labeled minimal unsatisfiable subset
// in which **no raw BDD variable index** appears — every facet/option in
// `core.conflicting_constraints[*].atoms[*]` (and the rejected atom) is a
// non-empty labeled `{facet}`/`{value}` string, never a bare integer
// (ADR-0031 D3/D4). The epic does not close until this test is green and
// pinned in CI.
//
// # Fixture (the configflux-vwhj synthetic CCM)
//
// Models the committed shared fixture
// `compiler/scenarios/s_labeled_mus/{00_definitions,10_components}.json` — a
// THREE-facet model (`cpu`, `cooling`, `psu`) with a CROSS-FACET `requires`
// constraint: selecting `cpu.highperf` requires `cooling.liquid` (and
// `psu.gold`). The interpreter (configflux-vwhj `int_*`) and runtime
// (`run_*`) E2E tests compile that committed scenario through the real
// `compile_model -> .ccm` path; this solver-level test hand-rolls a
// byte-for-byte v2 single-partition `.ccm` carrying the SAME semantics,
// because the `solver` test crate may not depend on `compiler` (ADR-0003 §2)
// and therefore cannot run the compiler. The hand-rolled BDD is the exact
// same shape the other solver explain fixtures use
// (`explain_rejection_mus.rs` / `explain_rejection_real_ccm.rs` /
// `apply_retract.rs`), built per ADR-0005 §§2-5.
//
// Compiled as an EXTERNAL crate over the public `solver` API, so a
// visibility or re-export regression on the labeled-core surface breaks this
// test even when every in-crate unit test still passes.
//
// Never imports `oxidd::*` or `batsat::*` (ADR-0003 §2/§3, ADR-0004 §4).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;

use solver::{CoreConstraintKind, LabeledAtom, OxiddBackend, Session};

// Re-declared ADR-0005 §4 constants, local so this test does not depend on
// `ccm_format`'s `pub(crate)` surface; drift surfaces as a fixture that no
// longer parses (the same contract every other solver fixture test follows).
const CCM_BDD_BIN_MAGIC: &[u8; 4] = b"CCMB";
const CCM_BDD_BIN_VERSION: u8 = 0x01;
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

#[path = "fixture_v2.rs"]
mod fixture_v2;

// ---------------------------------------------------------------------------
// BDD + symbols builders
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

/// `ccm.bdd.bin` for the feasible space of the labeled-MUS fixture under var
/// order `[cpu.highperf=0, cooling.liquid=1, cooling.air=2, psu.gold=3]`.
///
/// The feasible Boolean function is
/// ```text
///   F = (¬cpu.highperf ∨ cooling.liquid)   // highperf REQUIRES liquid
///     ∧ (¬cpu.highperf ∨ psu.gold)         // highperf REQUIRES gold
///     ∧ (cooling.liquid ⊻ cooling.air)     // exactly one cooling option
/// ```
/// Reduced ROBDD node table (index 0 = ⊥, 1 = ⊤; children referenced by the
/// real-emitter table-index form, 0 = ⊥ / 1 = ⊤, which the explain walker
/// accepts per configflux-autp):
/// ```text
///   2: psu.gold       { var=3, low=⊥,  high=⊤ }
///   3: ¬air∧gold      { var=2, low=2,  high=⊥ }
///   4: H (cpu=1)      { var=1, low=⊥,  high=3 }   // liquid ∧ ¬air ∧ gold
///   5: cooling.air    { var=2, low=⊥,  high=⊤ }
///   6: ¬cooling.air   { var=2, low=⊤,  high=⊥ }
///   7: G (cpu=0)      { var=1, low=5,  high=6 }   // liquid ⊻ air
///   8: root           { var=0, low=7,  high=4 }
/// ```
/// Falsifying paths make `cpu.highperf` REQUIRE `cooling.liquid` and forbid
/// `cooling.air` whenever `cooling.liquid` holds — so pinning `cpu.highperf`
/// and explaining `cooling.air` is a genuine cross-facet conflict.
fn build_labeled_mus_bdd_bin() -> Vec<u8> {
    let mut b = Vec::new();
    bdd_header(&mut b, 4, 9, 8);
    push_node(&mut b, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE); // 0: ⊥
    push_node(&mut b, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE); // 1: ⊤
    push_node(&mut b, 3, 0, 1); // 2: psu.gold
    push_node(&mut b, 2, 2, 0); // 3: ¬cooling.air ∧ psu.gold
    push_node(&mut b, 1, 0, 3); // 4: H — cpu.highperf cofactor
    push_node(&mut b, 2, 0, 1); // 5: cooling.air
    push_node(&mut b, 2, 1, 0); // 6: ¬cooling.air
    push_node(&mut b, 1, 5, 6); // 7: G — ¬cpu.highperf cofactor (liquid ⊻ air)
    push_node(&mut b, 0, 7, 4); // 8: root
    b
}

#[derive(Serialize)]
struct SymbolsOut {
    schema_version: u32,
    variable_order: Vec<String>,
    facet_to_var: BTreeMap<String, u32>,
    var_to_label: Vec<String>,
}

/// `ccm.symbols.json` for the labeled-MUS fixture: three facets
/// (`cpu`, `cooling`, `psu`), four option symbols, var order
/// `[cpu.highperf=0, cooling.liquid=1, cooling.air=2, psu.gold=3]`.
fn build_labeled_mus_symbols_json() -> Vec<u8> {
    let mut facet_to_var = BTreeMap::new();
    facet_to_var.insert("cpu.highperf".to_string(), 0u32);
    facet_to_var.insert("cooling.liquid".to_string(), 1u32);
    facet_to_var.insert("cooling.air".to_string(), 2u32);
    facet_to_var.insert("psu.gold".to_string(), 3u32);
    let sym = SymbolsOut {
        schema_version: 2,
        variable_order: vec![
            "cpu.highperf".to_string(),
            "cooling.liquid".to_string(),
            "cooling.air".to_string(),
            "psu.gold".to_string(),
        ],
        facet_to_var,
        var_to_label: vec![
            "cpu=highperf".to_string(),
            "cooling=liquid".to_string(),
            "cooling=air".to_string(),
            "psu=gold".to_string(),
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
    fixture_v2::unique_temp_dir("configflux-solver-labeled-mus", test_name)
}

fn load_labeled_mus_session(label: &str) -> Session<OxiddBackend> {
    let base = tempdir_for(label);
    let bound = "ab".repeat(32);
    let bdd_bytes = build_labeled_mus_bdd_bin();
    let symbols_bytes = build_labeled_mus_symbols_json();
    let dir: PathBuf = fixture_v2::materialize_single_partition_ccm(
        &base,
        &bdd_bytes,
        &symbols_bytes,
        &bound,
        4,
        9,
    );
    let ccm = Session::<OxiddBackend>::load_ccm(&dir).expect("load labeled-MUS fixture");
    Session::<OxiddBackend>::new(ccm).expect("session from labeled-MUS fixture")
}

// ---------------------------------------------------------------------------
// The labeled-MUS invariant assertion (the configflux-osp exit criterion)
// ---------------------------------------------------------------------------

/// A label is a non-empty `String` that is NOT a bare integer. The
/// `explain_rejection` contract forbids raw BDD/batsat variable indices in any
/// returned value (ADR-0031 D3/D4): a labeled name like `cooling` or `air`
/// passes; a stringified index like `"0"` or `"3"` fails. This is the literal
/// statement of the M4 hard exit criterion.
fn assert_is_label(s: &str, context: &str) {
    assert!(!s.is_empty(), "{context}: label must be non-empty");
    assert!(
        s.parse::<i64>().is_err(),
        "{context}: '{s}' is a bare integer — a raw BDD variable index leaked \
         into the labeled core (the configflux-osp exit criterion forbids this)",
    );
    assert!(
        s.chars().any(|c| c.is_ascii_alphabetic()),
        "{context}: '{s}' has no alphabetic character — not a labeled name",
    );
}

fn assert_atom_labeled(atom: &LabeledAtom, context: &str) {
    assert_is_label(&atom.facet, &format!("{context}.facet"));
    assert_is_label(&atom.value, &format!("{context}.value"));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// THE exit-criterion test. Over the synthetic ≥3-facet CCM with a cross-facet
/// `requires`, pinning `cpu.highperf` then explaining `cooling.air` is a
/// genuine conflict; the returned MUS must be labeled end-to-end with NO bare
/// integer anywhere in `conflicting_constraints[*].atoms[*]`.
///
/// This test MUST FAIL if `Session::explain_rejection` is reverted to the
/// stub (`RejectionExplanation { would_reject: false, core: None }` /
/// `RejectionExplanation::default()`): the stub returns `would_reject: false`
/// with no core, so the `would_reject` assertion and the `core` unwrap below
/// both fail. Verified locally per the configflux-vwhj acceptance criterion.
#[test]
fn labeled_mus_cross_facet_requires_conflict_has_no_raw_indices() {
    let mut session = load_labeled_mus_session("cross_facet_requires");

    // The model has THREE facets — proves the "≥3 facets" fixture premise.
    let facets: std::collections::BTreeSet<String> = session
        .ccm()
        .symbols()
        .expect("the fixture is symbolled")
        .variable_order()
        .filter_map(|sym| sym.rsplit_once('.').map(|(facet, _)| facet.to_string()))
        .collect();
    assert!(
        facets.len() >= 3,
        "the labeled-MUS fixture must carry at least three facets; got {facets:?}",
    );
    assert!(
        facets.contains("cpu") && facets.contains("cooling") && facets.contains("psu"),
        "the fixture must model the cpu/cooling/psu facets; got {facets:?}",
    );

    // Pin cpu.highperf. The cross-facet `requires` forces cooling.liquid (and
    // psu.gold), which under exactly-one forbids cooling.air.
    session
        .apply("cpu", "highperf")
        .expect("apply(cpu, highperf) must succeed — it is a feasible selection");

    // Explain cooling.air: in-domain, but unsatisfiable under the highperf pin.
    let explanation = session
        .explain_rejection("cooling", "air")
        .expect("explain_rejection on a modeled cross-facet conflict must return Ok");

    // The stub returns would_reject:false / no core — these two assertions are
    // the stub-reversion tripwires.
    assert!(
        explanation.would_reject,
        "cooling.air must be rejected after pinning cpu.highperf (cross-facet \
         requires) — a would_reject:false here means explain_rejection is the stub",
    );
    let core = explanation
        .core
        .as_ref()
        .expect(
            "a rejected option MUST carry a labeled core — a None core here means \
             explain_rejection is the stub",
        );

    // The rejected atom is the candidate, labeled — never a raw index.
    assert_eq!(
        core.rejected,
        LabeledAtom {
            facet: "cooling".to_string(),
            value: "air".to_string(),
        },
        "the rejected atom must be the candidate cooling.air",
    );
    assert_atom_labeled(&core.rejected, "core.rejected");
    assert!(core.minimal, "M4 deletion-based extraction reports a minimal core");

    // THE INVARIANT: a non-empty MUS, every atom in every conflicting
    // constraint a labeled name, no bare integer anywhere.
    assert!(
        !core.conflicting_constraints.is_empty(),
        "a genuine cross-facet conflict must name at least one conflicting constraint",
    );
    for (i, c) in core.conflicting_constraints.iter().enumerate() {
        assert!(
            matches!(
                c.kind,
                CoreConstraintKind::Selection | CoreConstraintKind::ModelRule
            ),
            "constraint[{i}] kind must be a labeled Selection or ModelRule",
        );
        assert!(
            !c.atoms.is_empty(),
            "constraint[{i}] must name at least one labeled atom",
        );
        for (j, atom) in c.atoms.iter().enumerate() {
            assert_atom_labeled(atom, &format!("constraint[{i}].atoms[{j}]"));
        }
    }

    // The cross-facet `requires` must surface: some conflicting constraint
    // names BOTH cpu.highperf and a cooling option — proving the MUS reflects
    // the genuine cross-facet relation, not a single-facet artifact.
    let names_cross_facet = core.conflicting_constraints.iter().any(|c| {
        let has_cpu = c.atoms.iter().any(|a| a.facet == "cpu");
        let has_cooling = c.atoms.iter().any(|a| a.facet == "cooling");
        has_cpu && has_cooling
    });
    assert!(
        names_cross_facet,
        "the MUS must name a cross-facet relation between cpu and cooling; got {:?}",
        core.conflicting_constraints,
    );

    // The prior pin must surface as a labeled Selection naming cpu.highperf —
    // proving the core reflects the replayed selection state.
    let names_pin = core.conflicting_constraints.iter().any(|c| {
        c.kind == CoreConstraintKind::Selection
            && c.atoms
                .iter()
                .any(|a| a.facet == "cpu" && a.value == "highperf")
    });
    assert!(
        names_pin,
        "the MUS must name the conflicting prior selection cpu.highperf; got {:?}",
        core.conflicting_constraints,
    );
}

/// A genuinely valid option returns `would_reject:false` and NO core
/// (ADR-0030 D5) — the negative companion that keeps the invariant honest
/// (the exit criterion is about labeled cores when a core exists, not about
/// inventing cores). On a fresh session `cooling.liquid` is a valid choice.
#[test]
fn labeled_mus_valid_option_returns_no_core() {
    let session = load_labeled_mus_session("valid_option");
    let explanation = session
        .explain_rejection("cooling", "liquid")
        .expect("explain_rejection on a valid option must return Ok");
    assert!(
        !explanation.would_reject,
        "cooling.liquid is valid on a fresh session — must not be rejected",
    );
    assert!(
        explanation.core.is_none(),
        "a genuinely valid option must carry no core, got {:?}",
        explanation.core,
    );
}
