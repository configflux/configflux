// SPDX-License-Identifier: BUSL-1.1
//
// `Session::explain_rejection` labeled-MUS extraction — configflux-kv5d.
//
// Exercises the real ADR-0004 §4 deletion-based MUS extraction end-to-end
// through the public `solver` API (compiled as an external crate, so a
// visibility or re-export regression breaks this test even when every
// in-crate test still passes). The fixtures are hand-rolled v2 single- and
// multi-partition CCM directories built byte-for-byte in a tempdir per
// ADR-0005 §§2-5, identical in shape to `apply_retract.rs` /
// `multi_partition_session.rs`.
//
// Acceptance coverage (issue configflux-kv5d):
//   (1) minimal 2-option / 1-constraint model: after `apply(engine, v6)`,
//       `explain_rejection(engine, v8)` returns `would_reject: true` and a
//       core whose conflicting constraints contain exactly the exclusion
//       `engine.v6 ⊻ engine.v8`, labeled — never raw integers;
//   (2) a genuinely valid `(facet, option)` returns `would_reject: false`
//       and NO core;
//   (3) no batsat type and no raw BDD/batsat variable index appears in any
//       returned value — enforced structurally (the public surface mentions
//       only labeled `String`s) and asserted here (every facet/value is a
//       non-empty label with no bare integer);
//   (4) multi-partition sessions: the composite formula across all
//       partitions is the MUS input — a cross-partition conflict is
//       explained with labeled names.
//
// Never imports `oxidd::*` or `batsat::*` (ADR-0003 §2/§3, ADR-0004 §4).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::Serialize;

use solver::{
    CoreConstraintKind, Error, LabeledAtom, LabeledConstraint, OxiddBackend, Session,
};

// Re-declared ADR-0005 §4 constants, local so this test does not depend on
// `ccm_format`'s `pub(crate)` surface; drift surfaces as a fixture that no
// longer parses (the same contract every other fixture test follows).
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

/// `ccm.bdd.bin` for `engine.v6 XOR engine.v8` under var order [v6=0, v8=1].
///
///   0: ⊥, 1: ⊤,
///   2: ¬v8  { var=1, low=⊤, high=⊥ }   (cofactor when v6=1)
///   3:  v8  { var=1, low=⊥, high=⊤ }   (cofactor when v6=0)
///   4:  v6  { var=0, low=3, high=2 }
///
/// Root = 4. Falsifying paths: (v6=0,v8=0) → `v6 ∨ v8`; (v6=1,v8=1) →
/// `¬v6 ∨ ¬v8`. The exactly-one constraint, one model rule with two halves.
fn build_xor_bdd_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    bdd_header(&mut bytes, 2, 5, 4);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    push_node(&mut bytes, 1, TERMINAL_TRUE, TERMINAL_FALSE); // ¬v8
    push_node(&mut bytes, 1, TERMINAL_FALSE, TERMINAL_TRUE); // v8
    push_node(&mut bytes, 0, 3, 2); // v6
    bytes
}

#[derive(Serialize)]
struct SymbolsOut {
    schema_version: u32,
    variable_order: Vec<String>,
    facet_to_var: BTreeMap<String, u32>,
    var_to_label: Vec<String>,
}

/// `ccm.symbols.json` for the two-variable XOR fixture.
fn build_xor_symbols_json() -> Vec<u8> {
    let mut facet_to_var = BTreeMap::new();
    facet_to_var.insert("engine.v6".to_string(), 0u32);
    facet_to_var.insert("engine.v8".to_string(), 1u32);
    let sym = SymbolsOut {
        schema_version: 2,
        variable_order: vec!["engine.v6".to_string(), "engine.v8".to_string()],
        facet_to_var,
        var_to_label: vec!["engine=v6".to_string(), "engine=v8".to_string()],
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
        "configflux-solver-explain-mus-{}-{}",
        test_name,
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&base);
    fs::create_dir_all(&base).expect("mkdir tempdir");
    base
}

fn load_xor_session(label: &str) -> Session<OxiddBackend> {
    let base = tempdir_for(label);
    let bound = "aa".repeat(32);
    let bdd_bytes = build_xor_bdd_bin();
    let symbols_bytes = build_xor_symbols_json();
    let dir: PathBuf = fixture_v2::materialize_single_partition_ccm(
        &base,
        &bdd_bytes,
        &symbols_bytes,
        &bound,
        2,
        5,
    );
    let ccm = Session::<OxiddBackend>::load_ccm(&dir).expect("load xor fixture");
    Session::<OxiddBackend>::new(ccm).expect("session from xor fixture")
}

// ---------------------------------------------------------------------------
// Assertions shared by the labeled-output tests
// ---------------------------------------------------------------------------

/// A label is a non-empty `String` that is not a bare integer. The
/// `explain_rejection` contract forbids raw BDD/batsat variable indices in
/// any returned value (ADR-0031 D3/D4); a labeled name like `engine` or `v8`
/// passes, a stringified index like `"0"` fails.
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

fn assert_atom_labeled(atom: &LabeledAtom, context: &str) {
    assert_is_label(&atom.facet, &format!("{context}.facet"));
    assert_is_label(&atom.value, &format!("{context}.value"));
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn minimal_two_option_one_constraint_returns_the_conflicting_constraint() {
    // Acceptance (1): minimal model, after pinning v6 the option v8 is
    // rejected; the MUS contains exactly the exclusion constraint, labeled.
    let mut session = load_xor_session("minimal_conflict");

    // Pin engine.v6. Under exactly-one this forces engine.v8 = 0.
    session
        .apply("engine", "v6")
        .expect("apply(engine, v6) must succeed");

    let explanation = session
        .explain_rejection("engine", "v8")
        .expect("explain_rejection on a modeled conflict must return Ok");

    assert!(
        explanation.would_reject,
        "engine.v8 must be rejected after pinning engine.v6 under exactly-one"
    );
    let core = explanation
        .core
        .as_ref()
        .expect("a rejected option must carry a labeled core");

    // The rejected atom is the candidate, labeled.
    assert_eq!(
        core.rejected,
        LabeledAtom {
            facet: "engine".to_string(),
            value: "v8".to_string(),
        },
        "the rejected atom must be the candidate engine.v8",
    );
    assert_atom_labeled(&core.rejected, "core.rejected");
    assert!(core.minimal, "the core must be reported minimal after shrinking");

    // Every atom in every conflicting constraint must be labeled.
    assert!(
        !core.conflicting_constraints.is_empty(),
        "a rejection must name at least one conflicting constraint"
    );
    for (i, c) in core.conflicting_constraints.iter().enumerate() {
        for (j, atom) in c.atoms.iter().enumerate() {
            assert_atom_labeled(atom, &format!("constraint[{i}].atoms[{j}]"));
        }
    }

    // The exclusion model rule `engine.v6 ⊻ engine.v8` must appear: a
    // ModelRule constraint naming both engine.v6 and engine.v8. (The prior
    // selection engine.v6 may also appear as a Selection constraint; the
    // acceptance bar is that the conflicting model constraint is present and
    // the core is minimal — no spurious extra model rules.)
    let exclusion = LabeledConstraint {
        kind: CoreConstraintKind::ModelRule,
        atoms: vec![
            LabeledAtom {
                facet: "engine".to_string(),
                value: "v6".to_string(),
            },
            LabeledAtom {
                facet: "engine".to_string(),
                value: "v8".to_string(),
            },
        ],
    };
    assert!(
        core.conflicting_constraints.contains(&exclusion),
        "the MUS must contain the exclusion model rule {{engine.v6, engine.v8}}; got {:?}",
        core.conflicting_constraints,
    );

    // Minimality at the model-rule level: exactly one model rule (the
    // exclusion) — the at-least-one half `v6 ∨ v8` is satisfied by the pin
    // and must have been shrunk away.
    let model_rules: Vec<&LabeledConstraint> = core
        .conflicting_constraints
        .iter()
        .filter(|c| c.kind == CoreConstraintKind::ModelRule)
        .collect();
    assert_eq!(
        model_rules.len(),
        1,
        "exactly one model rule (the exclusion) must remain after shrinking; got {model_rules:?}",
    );
}

#[test]
fn genuinely_valid_option_returns_no_core() {
    // Acceptance (2): on a fresh session (nothing pinned) engine.v8 is a
    // perfectly valid choice under exactly-one — explain must report
    // would_reject:false with no core.
    let session = load_xor_session("valid_v8");
    let explanation = session
        .explain_rejection("engine", "v8")
        .expect("explain_rejection on a valid option must return Ok");
    assert!(
        !explanation.would_reject,
        "engine.v8 is valid on a fresh session — must not be rejected"
    );
    assert!(
        explanation.core.is_none(),
        "a genuinely valid option must carry no core, got {:?}",
        explanation.core,
    );
}

#[test]
fn valid_option_after_a_compatible_pin_returns_no_core() {
    // A second valid-path check: pin engine.v6, then explain engine.v6
    // itself (the still-valid surviving option). It must not be rejected and
    // must carry no core.
    let mut session = load_xor_session("valid_after_pin");
    session
        .apply("engine", "v6")
        .expect("apply(engine, v6) must succeed");
    let explanation = session
        .explain_rejection("engine", "v6")
        .expect("explain on the surviving option must return Ok");
    assert!(
        !explanation.would_reject,
        "engine.v6 remains valid after pinning engine.v6 (idempotent)"
    );
    assert!(explanation.core.is_none(), "valid option ⇒ no core");
}

#[test]
fn unknown_facet_and_option_are_typed_errors_not_a_core() {
    // Division-of-labor (ADR-0030 D5): an unknown option under a known facet
    // is UnknownOption; a wholly unknown facet is UnknownFacet. Neither is a
    // MUS fault and neither carries a core.
    let session = load_xor_session("unknown");
    let unknown_option = session.explain_rejection("engine", "v12");
    assert!(
        matches!(&unknown_option, Err(Error::UnknownOption { facet, value }) if facet == "engine" && value == "v12"),
        "unknown option under a known facet must be UnknownOption, got {unknown_option:?}",
    );
    let unknown_facet = session.explain_rejection("transmission", "auto");
    assert!(
        matches!(&unknown_facet, Err(Error::UnknownFacet(f)) if f == "transmission"),
        "wholly unknown facet must be UnknownFacet, got {unknown_facet:?}",
    );
}

// ---------------------------------------------------------------------------
// Multi-partition: the composite formula is the MUS input (Q3)
// ---------------------------------------------------------------------------

/// A single-variable BDD forcing `var0 = 1` (a mandatory unit). Used as a
/// bridge partition that makes `region.a` mandatory:
///
///   0: ⊥, 1: ⊤, 2: { var=0, low=⊥, high=⊤ }   root = 2.
///
/// Falsifying path: (var0=0) → clause `(var0)`.
fn build_force_var0_true_bdd_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    bdd_header(&mut bytes, 1, 3, 2);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    push_node(&mut bytes, 0, TERMINAL_FALSE, TERMINAL_TRUE);
    bytes
}

/// A single-variable BDD forbidding `var0 = 1` (i.e. forcing `var0 = 0`):
///
///   0: ⊥, 1: ⊤, 2: { var=0, low=⊤, high=⊥ }   root = 2.
///
/// Falsifying path: (var0=1) → clause `(¬var0)`.
fn build_forbid_var0_bdd_bin() -> Vec<u8> {
    let mut bytes = Vec::new();
    bdd_header(&mut bytes, 1, 3, 2);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_FALSE, TERMINAL_FALSE);
    push_node(&mut bytes, TERMINAL_VAR_INDEX, TERMINAL_TRUE, TERMINAL_TRUE);
    push_node(&mut bytes, 0, TERMINAL_TRUE, TERMINAL_FALSE);
    bytes
}

fn single_var_symbols_json(symbol: &str, label: &str) -> Vec<u8> {
    let mut facet_to_var = BTreeMap::new();
    facet_to_var.insert(symbol.to_string(), 0u32);
    let sym = SymbolsOut {
        schema_version: 2,
        variable_order: vec![symbol.to_string()],
        facet_to_var,
        var_to_label: vec![label.to_string()],
    };
    let mut v = serde_json::to_vec(&sym).expect("symbols serialize");
    v.push(b'\n');
    v
}

#[test]
fn multi_partition_composite_formula_is_explained_with_labels() {
    // Acceptance (4): a cluster partition makes `power.grid` MANDATORY
    // (cluster BDD forces power.grid=1), and a bridge partition FORBIDS
    // `power.grid` (bridge BDD forces power.grid=0). The two partitions
    // share the symbol `power.grid` by name (Q3 — identity is by name across
    // partitions). The composite is already unsatisfiable for power.grid=1
    // via the bridge, so explaining the option `power.grid` surfaces the
    // bridge's forbidding rule as a labeled model constraint over the
    // composite — proving the MUS input is the composite, not one partition.
    let base = tempdir_for("multi_partition");
    let bound = "bb".repeat(32);

    let cluster_bdd = build_force_var0_true_bdd_bin();
    let cluster_syms = single_var_symbols_json("power.grid", "power=grid");
    let bridge_bdd = build_forbid_var0_bdd_bin();
    let bridge_syms = single_var_symbols_json("power.grid", "power=grid");
    // Top-level union symbols: the single shared symbol.
    let union_syms = single_var_symbols_json("power.grid", "power=grid");

    let cluster = fixture_v2::PartitionPayload {
        bdd_bytes: &cluster_bdd,
        symbols_bytes: &cluster_syms,
        var_count: 1,
        node_count: 3,
    };
    let bridge = fixture_v2::PartitionPayload {
        bdd_bytes: &bridge_bdd,
        symbols_bytes: &bridge_syms,
        var_count: 1,
        node_count: 3,
    };

    let dir: PathBuf = fixture_v2::materialize_multi_partition_ccm(
        &base,
        &bound,
        1, // top var_count
        3, // top node_count
        &union_syms,
        &[cluster],
        Some(&bridge),
    );
    let ccm = Session::<OxiddBackend>::load_ccm(&dir).expect("load multi-partition fixture");
    let session = Session::<OxiddBackend>::new(ccm).expect("multi-partition session");

    // Explain the option `power.grid`: the bridge forbids it, so the
    // composite rejects it. (No apply needed — the conflict is structural in
    // the composite formula.)
    let explanation = session
        .explain_rejection("power", "grid")
        .expect("explain over the composite must return Ok");
    assert!(
        explanation.would_reject,
        "power.grid is forbidden by the bridge partition — the composite rejects it"
    );
    let core = explanation
        .core
        .as_ref()
        .expect("a composite rejection must carry a labeled core");

    assert_eq!(
        core.rejected,
        LabeledAtom {
            facet: "power".to_string(),
            value: "grid".to_string(),
        },
    );
    // The forbidding rule `(¬power.grid)` from the bridge must appear as a
    // labeled model rule naming power.grid — proving the bridge partition's
    // clause entered the composite CNF.
    let forbid = LabeledConstraint {
        kind: CoreConstraintKind::ModelRule,
        atoms: vec![LabeledAtom {
            facet: "power".to_string(),
            value: "grid".to_string(),
        }],
    };
    assert!(
        core.conflicting_constraints.contains(&forbid),
        "the composite MUS must contain the bridge's forbidding rule for power.grid; got {:?}",
        core.conflicting_constraints,
    );
    // No raw integers anywhere in the labeled output.
    for c in &core.conflicting_constraints {
        for atom in &c.atoms {
            assert_atom_labeled(atom, "composite.constraint.atom");
        }
    }
}
