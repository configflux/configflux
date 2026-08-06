// SPDX-License-Identifier: BUSL-1.1

//! Cross-heuristic Boolean-equivalence test for the CCM emitter.
//!
//! Pins ADR-0005 §6 R2: emitting the **same** [`ConditionModel`] under
//! `var_order_heuristic = "facet-name-ascending"` vs
//! `"clause-grouped-dfs"` MUST produce different `ccm.bdd.bin` bytes
//! (the order matters for byte representation) but encode the **same**
//! Boolean function (every facet's set of valid values is identical).
//!
//! The test does NOT assert byte-equality of `valid_options` results
//! (that would be over-specified — the per-heuristic emission is
//! free to surface options in any order). Instead, it set-compares
//! the option list per facet via `BTreeSet`.
//!
//! # Why a separate file (not extending `ccm_emitter_roundtrip.rs`)
//!
//! Keeps the existing pinned-order roundtrip test independent of the
//! heuristic switch. A regression in the heuristic plumbing must
//! surface here without dragging the legacy test target into the
//! same flake.

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use compiler::ccm_emitter::{emit_ccm_dir_with_heuristic, ConditionModel};
use solver::{OxiddBackend, Session};

/// Wire tag for the legacy default heuristic. Pinned by ADR-0005 §2.
const HEURISTIC_FACET_ASC: &str = "facet-name-ascending";
/// Wire tag for the opt-in clause-grouped-DFS heuristic. Pinned by
/// ADR-0005 §2.
const HEURISTIC_CLAUSE_DFS: &str = "clause-grouped-dfs";

/// Hand-built model with 12 features and 4 cross-tree implications
/// (>= 10 features, >= 3 cross-tree as bd-lxah requires).
///
/// **Clause order matters:** the cross-tree implications appear
/// FIRST so the two heuristics genuinely disagree on the BDD
/// variable order. With the implications first, clause-grouped-DFS
/// emits pairs in `(antecedent, consequent)` first-sight order:
/// `a, j, b, l, c, i, d, k, ...`. Facet-name-ascending always emits
/// alphabetical order: `a, b, c, d, ..., l`. The two orderings differ
/// → the emitted `ccm.bdd.bin` bytes differ → exactly the regime
/// ADR-0005 §6 R2 pins.
///
/// If you reorder clauses so the leading single-atom clauses come
/// first (`a == 'on'`, `b == 'on'`, ...), DFS first-sight order
/// degenerates to alphabetical and the test loses its discriminator.
/// That's a real risk: if a future refactor flattens clauses or
/// re-sorts them, this fixture must move with it.
fn small_model_with_cross_tree() -> ConditionModel {
    ConditionModel::from_clauses(
        // 32-byte hex digest required by `validate_hash`.
        "33".repeat(32),
        vec![
            // Cross-tree implications FIRST so DFS first-sight order
            // diverges from alphabetical. `!A || B` ≡ `A -> B`. Each
            // pair connects non-adjacent letters in the alphabet.
            "!(a == 'on') || (j == 'on')".to_string(),
            "!(b == 'on') || (l == 'on')".to_string(),
            "!(c == 'on') || (i == 'on')".to_string(),
            "!(d == 'on') || (k == 'on')".to_string(),
            // Single-atom clauses ensure every facet appears in the
            // symbol table even if not touched by an implication.
            "a == 'on'".to_string(),
            "b == 'on'".to_string(),
            "c == 'on'".to_string(),
            "d == 'on'".to_string(),
            "e == 'on'".to_string(),
            "f == 'on'".to_string(),
            "g == 'on' || h == 'on'".to_string(),
            "i == 'on'".to_string(),
            "j == 'on'".to_string(),
            "k == 'on'".to_string(),
            "l == 'on'".to_string(),
        ],
    )
}

/// All 12 facet tags appearing in the test model. Order does not
/// matter — this is the comparison key set.
const FACETS: &[&str] = &[
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l",
];

#[test]
fn cross_heuristic_boolean_equivalence_pins_adr_0005_r2() {
    let base = tempdir_for("ccm_emitter_equivalence");
    let dir_facet_asc = base.join("facet_asc");
    let dir_clause_dfs = base.join("clause_dfs");
    let model = small_model_with_cross_tree();

    // 1. Emit twice — once per heuristic.
    emit_ccm_dir_with_heuristic(&model, &dir_facet_asc, HEURISTIC_FACET_ASC)
        .expect("emit under facet-name-ascending");
    emit_ccm_dir_with_heuristic(&model, &dir_clause_dfs, HEURISTIC_CLAUSE_DFS)
        .expect("emit under clause-grouped-dfs");

    // 2. Different heuristics MUST produce different bytes — otherwise
    //    the heuristic plumbing is a no-op. This is the corollary
    //    half of ADR-0005 §6 R2: the choice changes the artifact, but
    //    not the function it encodes.
    //
    // configflux-mwyp / ADR-0012 Amendment 1 §11: under v2 the BDD
    // lives in `partition-0000/ccm.bdd.bin` for the single-partition
    // (collapse) case the FAMA-scale fixtures produce.
    let bytes_facet_asc = fs::read(dir_facet_asc.join("partition-0000").join("ccm.bdd.bin"))
        .expect("read partition-0000/ccm.bdd.bin under facet-name-ascending");
    let bytes_clause_dfs = fs::read(dir_clause_dfs.join("partition-0000").join("ccm.bdd.bin"))
        .expect("read partition-0000/ccm.bdd.bin under clause-grouped-dfs");
    assert_ne!(
        bytes_facet_asc, bytes_clause_dfs,
        "different heuristics must produce different ccm.bdd.bin bytes; \
         identical bytes would mean the heuristic switch is a no-op"
    );

    // 3. Load each emitted CCM into its own Session and gather the
    //    per-facet `valid_options` set under an unconstrained session
    //    (Boolean function = the BDD root itself).
    let session_facet_asc = load_session(&dir_facet_asc);
    let session_clause_dfs = load_session(&dir_clause_dfs);

    // 4. Per-facet set-equality. ORDER-INDEPENDENT — `valid_options`
    //    is free to return options in heuristic-dependent order.
    for facet in FACETS {
        let opts_asc: BTreeSet<String> = session_facet_asc
            .valid_options(facet)
            .unwrap_or_else(|e| panic!("valid_options('{facet}') under facet-asc: {e}"))
            .options
            .into_iter()
            .collect();
        let opts_dfs: BTreeSet<String> = session_clause_dfs
            .valid_options(facet)
            .unwrap_or_else(|e| panic!("valid_options('{facet}') under clause-dfs: {e}"))
            .options
            .into_iter()
            .collect();
        assert_eq!(
            opts_asc, opts_dfs,
            "facet '{facet}': valid_options diverged across heuristics — \
             ADR-0005 §6 R2 (Boolean equivalence) violated"
        );
    }
}

fn load_session(dir: &PathBuf) -> Session<OxiddBackend> {
    let ccm = Session::<OxiddBackend>::load_ccm(dir).expect("load CCM directory");
    Session::<OxiddBackend>::new(ccm).expect("deserialize emitted BDD")
}

// Collision-proof temp-dir naming shared across the compiler integration
// tests; see `temp_dirs.rs` (configflux-rvpb).
#[path = "temp_dirs.rs"]
mod temp_dirs;

fn tempdir_for(test_name: &str) -> PathBuf {
    temp_dirs::unique_temp_dir("configflux-compiler", test_name)
}
