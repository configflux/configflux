// SPDX-License-Identifier: BUSL-1.1
//
// CuddBackend parity integration test — configflux-jxk6
//
// This file mirrors `solver/tests/oxidd_backend_parity.rs` but for the
// CUDD-backed implementation of `SolverBackend` per ADR-0004 §3
// second-pass amendment 2026-05-09 (configflux-dwwv) and ADR-0003 §3.
//
// Three purposes:
//
// 1. Prove `CuddBackend` satisfies `SolverBackend` at compile time.
//    (`Session::<CuddBackend>::...` in an external-crate integration
//    test surfaces a trait-bound failure as a compile error, not a
//    runtime panic.)
// 2. Prove the public Session API does not leak CUDD types. This file
//    never imports `cudd_sys::*`; if a Session method signature ever
//    surfaced a `*mut DdNode` / `*mut DdManager` / `cudd_sys::*` type,
//    this file would stop compiling.
// 3. Prove semantic parity vs OxiddBackend on a small fixture. Two
//    backends fed the same canonical ADR-0005 §4 node table must agree
//    on every `is_var_sat_under(var)` query — that is the contract the
//    cycle-side ADR-0004 amendment relies on for swapping engines
//    without bumping `ccm.bdd.bin`'s on-disk schema.
//
// Note on test surface: this file does NOT exercise `Session::load_ccm`
// against a real on-disk CCM directory because the CUDD backend's job
// in jxk6 is the trait surface, not the format reader. The
// CCM-bytestream → CUDD translation pass is its own follow-on task
// (configflux-fvew). The `deserialize_bdd` call here directly feeds
// the `BddNode` table the loader would have produced, which is
// sufficient for parity proof.

use std::path::Path;

use solver::{
    BddNode, Ccm, CuddBackend, Error, OxiddBackend, RejectionExplanation, ResolveResult, Session,
    Snapshot, SolverBackend, ValidOptions, VariableOrder, CapacityHints,
};

/// Path passed to `Session::load_ccm`. The M0 `Ccm::load_from_cmp` stub
/// ignores its argument; downstream tasks will replace this with a
/// real `.cmp` path.
const UNUSED_CCM_PATH: &str = "unused/cudd_backend_parity.ccm";

/// `var_index` sentinel used for terminal records in
/// `ccm.bdd.bin` — copied here because the constant lives in the
/// crate-private `ccm_format` module and the integration test cannot
/// reach into it. The value is part of ADR-0005 §4 and is therefore
/// stable across solver versions; pinning the literal here is
/// deliberate.
const TERMINAL_VAR_INDEX: u32 = 0xFFFF_FFFF;
const TERMINAL_FALSE: u32 = 0xFFFF_FFFF;
const TERMINAL_TRUE: u32 = 0xFFFF_FFFE;

fn fresh_empty_cudd_session() -> Session<CuddBackend> {
    let ccm = Session::<CuddBackend>::load_ccm(Path::new(UNUSED_CCM_PATH))
        .expect("load_ccm on empty CCM must succeed per ADR-0003 §4");
    Session::<CuddBackend>::new(ccm)
        .expect("Session::new on empty CCM must succeed per ADR-0003 §4")
}

#[test]
fn cudd_backend_session_round_trip_matches_null_backend() {
    // Mirrors the ADR-0003 §4 call sequence in `round_trip_empty.rs`
    // but over CuddBackend. A regression in the wrapper surfaces here
    // as a shape mismatch, not a panic.
    let mut session = fresh_empty_cudd_session();

    let options: ValidOptions = session
        .valid_options("any-facet")
        .expect("valid_options on cudd-backed empty session must return Ok");
    assert_eq!(options.count, 0);

    session
        .apply("engine", "v6")
        .expect("apply on cudd-backed empty session must return Ok");
    session
        .retract("engine")
        .expect("retract on cudd-backed empty session must return Ok");

    // configflux-kv5d: real MUS extraction (ADR-0004 §4). On the cudd-backed
    // empty session there is no symbol table, so the candidate is "unknown" —
    // a typed `UnknownOption`, not the old stub `Ok`.
    let explanation = session.explain_rejection("engine", "v6");
    assert!(
        matches!(&explanation, Err(Error::UnknownOption { facet, value }) if facet == "engine" && value == "v6"),
        "explain_rejection on the empty cudd session must be UnknownOption, got {explanation:?}",
    );

    let resolved: ResolveResult = session
        .resolve()
        .expect("resolve on cudd-backed empty session must return Ok");
    assert!(resolved.satisfiable);
}

#[test]
fn cudd_backend_advertises_stable_backend_id() {
    // ADR-0005 §2 pins `ccm.manifest.json.algorithm` as the stable
    // backend identifier. Pinning the string here ensures the CCM
    // manifest writer (a future translation-pass task,
    // configflux-fvew) sees the expected tag; a silent CUDD bump that
    // changed this value would fail here before shipping.
    let session = fresh_empty_cudd_session();
    assert_eq!(
        session.backend().backend_id(),
        "cudd-v3.0",
        "cudd backend must advertise a stable backend_id tag"
    );
}

#[test]
fn cudd_backend_state_hash_is_stable_across_loads() {
    // ADR-0003 §4 makes `state_hash` a backend-agnostic byte-stability
    // property. Two fresh empty sessions — across the trait boundary —
    // must yield the same hash.
    let session_a = fresh_empty_cudd_session();
    let session_b = fresh_empty_cudd_session();
    assert_eq!(
        session_a.state_hash(),
        session_b.state_hash(),
        "state_hash must be byte-for-byte identical across two identical fresh loads of the cudd-backed empty CCM"
    );
}

// ---------------------------------------------------------------------
// Direct trait-level tests against the CuddBackend `SolverBackend`
// surface. These do not go through `Session` because the M0 `Session`
// path treats `Ccm::empty()` as a no-op for the BDD reconstruction —
// the trait-level path is what proves `deserialize_bdd`,
// `is_var_sat_under`, and `apply_and` work end-to-end and are
// semantically identical to OxiddBackend.
// ---------------------------------------------------------------------

/// Build the canonical 2-variable AND BDD: `f(a, b) = a ∧ b`.
///
/// Encoded post-order under ADR-0005 §4:
///
///   - index 0: terminal FALSE
///   - index 1: terminal TRUE
///   - index 2: non-terminal `{ var=1 (b), low=⊥, high=⊤ }`
///     — this is the b-cofactor under `a=1`: `(b ∧ ⊤) ∨ (¬b ∧ ⊥) = b`.
///   - index 3: non-terminal `{ var=0 (a), low=⊥, high=2 }`
///     — `(a ∧ b) ∨ (¬a ∧ ⊥) = a ∧ b`.
///
/// Returns the node table and the root index.
fn build_and_bdd_table() -> (Vec<BddNode>, u32) {
    let nodes = vec![
        BddNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_FALSE,
            high_id: TERMINAL_FALSE,
            flags: 0,
        },
        BddNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_TRUE,
            high_id: TERMINAL_TRUE,
            flags: 0,
        },
        BddNode {
            var_index: 1,
            low_id: 0,
            high_id: 1,
            flags: 0,
        },
        BddNode {
            var_index: 0,
            low_id: 0,
            high_id: 2,
            flags: 0,
        },
    ];
    (nodes, 3)
}

#[test]
fn cudd_backend_deserialize_bdd_round_trips_trivial_terminals() {
    // The fast path: a root sentinel (TERMINAL_TRUE / TERMINAL_FALSE)
    // must produce a constant handle without touching the node table.
    let mut backend = CuddBackend::new_session(VariableOrder::empty(), CapacityHints::default())
        .expect("new_session must succeed");

    let true_handle = backend
        .deserialize_bdd(0, &[], TERMINAL_TRUE)
        .expect("deserialize_bdd on TERMINAL_TRUE must succeed");
    assert!(backend.is_true(true_handle));
    assert!(!backend.is_false(true_handle));

    let false_handle = backend
        .deserialize_bdd(0, &[], TERMINAL_FALSE)
        .expect("deserialize_bdd on TERMINAL_FALSE must succeed");
    assert!(backend.is_false(false_handle));
    assert!(!backend.is_true(false_handle));
}

#[test]
fn cudd_backend_deserialize_bdd_rebuilds_non_trivial_and() {
    // Build `a ∧ b` and check the four cofactor SAT outcomes. This is
    // the smallest non-trivial case and exercises both
    // `deserialize_bdd` (variable allocation, ite walk, root push) and
    // `is_var_sat_under` (apply-cache cofactor check).
    let (nodes, root) = build_and_bdd_table();
    let mut backend = CuddBackend::new_session(VariableOrder::empty(), CapacityHints::default())
        .expect("new_session must succeed");
    let f_and = backend
        .deserialize_bdd(2, &nodes, root)
        .expect("deserialize_bdd on a∧b must succeed");

    // `a ∧ b` is satisfiable, so neither `f ∧ a` nor `f ∧ b` is ⊥.
    assert!(
        backend
            .is_var_sat_under(f_and, 0)
            .expect("is_var_sat_under(a) must succeed"),
        "f ∧ a must be SAT for f = a∧b"
    );
    assert!(
        backend
            .is_var_sat_under(f_and, 1)
            .expect("is_var_sat_under(b) must succeed"),
        "f ∧ b must be SAT for f = a∧b"
    );
    assert!(!backend.is_false(f_and), "f = a∧b is not the FALSE constant");
    assert!(!backend.is_true(f_and), "f = a∧b is not the TRUE constant");
}

#[test]
fn cudd_backend_apply_and_reduces_to_unit_clause() {
    // `apply_and(a∧b, a) = a∧b∧a = a∧b` (idempotent on a). The
    // resulting handle must still report SAT under both a and b.
    let (nodes, root) = build_and_bdd_table();
    let mut backend = CuddBackend::new_session(VariableOrder::empty(), CapacityHints::default())
        .expect("new_session must succeed");
    let f_and = backend
        .deserialize_bdd(2, &nodes, root)
        .expect("deserialize_bdd on a∧b must succeed");
    let f_and_a = backend
        .apply_and(f_and, 0)
        .expect("apply_and(a∧b, a) must succeed");
    assert!(!backend.is_false(f_and_a), "(a∧b)∧a = a∧b is not ⊥");
    assert!(
        backend
            .is_var_sat_under(f_and_a, 1)
            .expect("is_var_sat_under(b) must succeed on the applied handle"),
        "(a∧b)∧a∧b is SAT"
    );
}

#[test]
fn cudd_backend_matches_oxidd_backend_on_and_fixture() {
    // Semantic parity proof. Both backends are fed the identical
    // ADR-0005 §4 node table for `a ∧ b`; both must agree on every
    // `is_var_sat_under` query and on `is_true` / `is_false` for the
    // root. This is the headline contract of the ADR-0004 second-pass
    // amendment (configflux-dwwv): swapping the engine MUST NOT change
    // the answers solver-side, only the gate-time RSS / wall envelope.
    let (nodes, root) = build_and_bdd_table();

    let mut cudd = CuddBackend::new_session(VariableOrder::empty(), CapacityHints::default())
        .expect("cudd new_session");
    let mut oxidd = OxiddBackend::new_session(VariableOrder::empty(), CapacityHints::default())
        .expect("oxidd new_session");

    let cudd_root = cudd
        .deserialize_bdd(2, &nodes, root)
        .expect("cudd deserialize_bdd");
    let oxidd_root = oxidd
        .deserialize_bdd(2, &nodes, root)
        .expect("oxidd deserialize_bdd");

    // Constant predicates must agree: a∧b is neither TRUE nor FALSE.
    assert_eq!(
        cudd.is_true(cudd_root),
        oxidd.is_true(oxidd_root),
        "is_true must agree across backends on a∧b"
    );
    assert_eq!(
        cudd.is_false(cudd_root),
        oxidd.is_false(oxidd_root),
        "is_false must agree across backends on a∧b"
    );

    // Cofactor SAT must agree on every variable.
    for var_idx in 0..2u32 {
        let cudd_sat = cudd.is_var_sat_under(cudd_root, var_idx).expect("cudd cofactor");
        let oxidd_sat = oxidd
            .is_var_sat_under(oxidd_root, var_idx)
            .expect("oxidd cofactor");
        assert_eq!(
            cudd_sat, oxidd_sat,
            "cofactor SAT under x_{var_idx} must agree across backends on a∧b"
        );
    }

    // After applying a unit clause, cofactor SAT must still agree.
    let cudd_after = cudd.apply_and(cudd_root, 0).expect("cudd apply_and");
    let oxidd_after = oxidd.apply_and(oxidd_root, 0).expect("oxidd apply_and");
    for var_idx in 0..2u32 {
        let c = cudd
            .is_var_sat_under(cudd_after, var_idx)
            .expect("cudd cofactor post-apply");
        let o = oxidd
            .is_var_sat_under(oxidd_after, var_idx)
            .expect("oxidd cofactor post-apply");
        assert_eq!(
            c, o,
            "post-apply_and cofactor SAT under x_{var_idx} must agree across backends"
        );
    }
}

#[test]
fn cudd_backend_deserialize_bdd_rejects_out_of_range_var_index() {
    // Bad input: a node references var_index = 5 in a table that
    // declared var_count = 2. The error variant is internal to the
    // backend (Serialization), but the user-visible contract is "an
    // Err is returned and we do not segfault". CUDD's strictness
    // matters here because a forged var_index could otherwise produce
    // an out-of-bounds C-side memory read.
    let mut backend = CuddBackend::new_session(VariableOrder::empty(), CapacityHints::default())
        .expect("new_session must succeed");
    let nodes = vec![
        BddNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_FALSE,
            high_id: TERMINAL_FALSE,
            flags: 0,
        },
        BddNode {
            var_index: TERMINAL_VAR_INDEX,
            low_id: TERMINAL_TRUE,
            high_id: TERMINAL_TRUE,
            flags: 0,
        },
        // var_index = 5 is out of range for var_count = 2.
        BddNode {
            var_index: 5,
            low_id: 0,
            high_id: 1,
            flags: 0,
        },
    ];
    let err = backend
        .deserialize_bdd(2, &nodes, 2)
        .expect_err("out-of-range var_index must produce an Err, not a segfault");
    let _ = err; // any backend-level Err is acceptable; semantics not pinned here
}

#[test]
fn cudd_backend_deserialize_bdd_rejects_out_of_range_root() {
    // Bad input: root = 99 against a 4-entry table. Must surface as an
    // Err, not a panic or out-of-bounds C-side read. Mirrors the
    // OxiddBackend test of the same name in spirit.
    let mut backend = CuddBackend::new_session(VariableOrder::empty(), CapacityHints::default())
        .expect("new_session must succeed");
    let (nodes, _root) = build_and_bdd_table();
    let err = backend
        .deserialize_bdd(2, &nodes, 99)
        .expect_err("out-of-range root must produce an Err");
    let _ = err; // any Err is acceptable
}

// ---------------------------------------------------------------------
// Compile-time type-leak guards. Each of these is a `let _:` ascription
// that names a public type from the `solver` crate. If a future change
// to the public API surfaced a CUDD type, the ascription would fail to
// compile because `cudd_sys::*` is not in scope here.
// ---------------------------------------------------------------------

#[test]
fn public_api_does_not_leak_cudd_types() {
    // Construct a CuddBackend session and pull out every public-API
    // value it touches. None of these annotations import from
    // `cudd_sys::*`; if a method signature ever leaked a CUDD type,
    // this function would stop compiling before it ran.
    let session = fresh_empty_cudd_session();
    let _id: &'static str = session.backend().backend_id();
    let _ccm: &Ccm = session.ccm();
    // FormulaHandle is a pub solver type with private fields; we cannot
    // construct one in an external crate, but we can prove no method on
    // the Session API surfaces a CUDD type by exercising the rest of
    // the public surface below.
    // Snapshot, ValidOptions, ResolveResult, RejectionExplanation are
    // already exercised in the round-trip test above; pinning the
    // ascriptions here keeps the type-leak guard self-contained.
    let _snap: Snapshot = Snapshot::empty();
    let _opts: ValidOptions = ValidOptions::default();
    let _res: ResolveResult = ResolveResult::default();
    let _rej: RejectionExplanation = RejectionExplanation::default();
}
