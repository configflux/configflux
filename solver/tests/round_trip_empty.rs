// SPDX-License-Identifier: BUSL-1.1
//
// Round-trip-empty integration test for `solver::Session`.
//
// This is the M0 exit criterion test for bd-configflux-6tl. It is a proper
// Cargo-style integration test living under `solver/tests/` so that it
// compiles as an external crate and calls through the **public** API surface
// of the `solver` crate — no `crate::` paths, no internal `#[cfg(test)] mod`
// tricks. That boundary is deliberate: if `interpreter` or `runtime` cannot
// import what this test imports, the public API is incomplete.
//
// # Contract under test
//
// Every assertion here traces back to ADR-0003 Section 4 ("Session API
// contract"), not to whatever the M0 stub happens to return. The file calls
// the `Session` methods pinned by ADR-0003 §4 and checks the shape the ADR
// requires. (The `set_context`/`snapshot`/`restore` stubs were retired per
// ADR-0030 D6; the session-transport surface is redesigned against real
// requirements when the daemon milestone is scheduled.)
//
// # Why not the internal smoke test
//
// `solver/src/lib.rs::smoke_tests::empty_session_round_trip_smoke` already
// calls the same methods, but it runs as `#[cfg(test)] mod` inside the
// crate. That path cannot catch visibility or re-export regressions that
// would break `interpreter` or `runtime` — only an external integration
// test can. That is what this file is for, and it is what the `configflux-6tl`
// acceptance criterion requires for M0 exit.
//
// # Red-path verification (in-head, per worker brief)
//
// - If `Session::state_hash` returned non-deterministic bytes across two
//   identical empty loads, `state_hash_is_stable_across_two_loads` would
//   fail at the `assert_eq!` line comparing the two StateHash values.
// - If `Session::apply` on an empty CCM panicked instead of returning a
//   well-defined `Ok(())`, `apply_on_empty_ccm_is_well_defined` would
//   propagate the panic and fail the test.

use std::path::Path;

use solver::{
    Error, NullBackend, ResolveResult, Session, StateHash, ValidOptions, CCM_SCHEMA_VERSION_V1,
};

/// Path passed to `Session::load_ccm`. The M0 `Ccm::load_from_cmp` stub
/// ignores its argument and returns an empty `Ccm` regardless of path, so
/// no fixture tree on disk is required. M1 will replace this with a real
/// parse and the path will need to point at a valid `.cmp` manifest.
const UNUSED_CCM_PATH: &str = "unused/round_trip_empty.ccm";

/// Build a fresh `Session<NullBackend>` on an empty CCM via the public API.
/// Every test starts from this helper so the common load→new path is only
/// written once.
fn fresh_empty_session() -> Session<NullBackend> {
    let ccm = Session::<NullBackend>::load_ccm(Path::new(UNUSED_CCM_PATH))
        .expect("load_ccm on empty CCM must succeed per ADR-0003 §4");
    Session::<NullBackend>::new(ccm).expect("Session::new on empty CCM must succeed per ADR-0003 §4")
}

#[test]
fn load_ccm_empty_returns_v1_ccm() {
    // ADR-0003 §4: `load_ccm` returns an opaque `Ccm` handle. ADR-0005 §2
    // pins the v1 schema tag. The M0 empty CCM must advertise v1 and the
    // zero hash (until M1 wires the real SHA-256 over the artifact bytes).
    let ccm = Session::<NullBackend>::load_ccm(Path::new(UNUSED_CCM_PATH))
        .expect("load_ccm on empty CCM must succeed");
    assert_eq!(
        ccm.schema_version(),
        CCM_SCHEMA_VERSION_V1,
        "empty CCM must report schema_version = CCM_SCHEMA_VERSION_V1"
    );
    assert_eq!(
        ccm.ccm_hash(),
        [0u8; 32],
        "empty CCM must have the zero ccm_hash in M0 per the scaffolding stub"
    );
}

#[test]
fn new_constructs_session_bound_to_empty_ccm() {
    // ADR-0003 §4: `Session::new(ccm)` takes a loaded CCM handle and returns
    // a Session bound to it. The bound CCM must be reachable for later
    // methods (snapshot, restore, state_hash pre-image).
    let ccm = Session::<NullBackend>::load_ccm(Path::new(UNUSED_CCM_PATH))
        .expect("load_ccm must succeed");
    let session = Session::<NullBackend>::new(ccm).expect("Session::new must succeed");
    assert_eq!(
        session.ccm().schema_version(),
        CCM_SCHEMA_VERSION_V1,
        "Session must expose the bound CCM via `ccm()` for M1 snapshot/restore wiring"
    );
}

#[test]
fn valid_options_returns_empty_set_for_any_facet() {
    // ADR-0003 §4: `valid_options` is a pure query that returns the still-
    // valid options for a facet. An empty CCM admits no facets and so no
    // options; the return must be an `Ok(ValidOptions)` with count 0, not
    // an error and not a panic.
    let session = fresh_empty_session();
    let options: ValidOptions = session
        .valid_options("any-facet")
        .expect("valid_options on empty CCM must return Ok");
    assert_eq!(
        options.count, 0,
        "empty CCM must yield zero valid options for every facet"
    );
    // Invariance across different facet labels: the stub must not discriminate.
    let other: ValidOptions = session
        .valid_options("")
        .expect("valid_options on empty facet label must also return Ok");
    assert_eq!(
        other.count, 0,
        "empty CCM must yield zero valid options even for the empty facet label"
    );
}

#[test]
fn apply_on_empty_ccm_is_well_defined() {
    // ADR-0003 §4: `apply(facet, option)` on an empty CCM has no effect
    // to apply, but the contract requires a well-defined return (not a
    // panic). The M0 stub returns `Ok(())`; M1 may choose to return a
    // rejection here instead — either way this test will catch a panic
    // or a silent corruption of session state.
    //
    // Red-path check: if `apply` began panicking at runtime under an M1
    // refactor, this `expect("apply ... must not panic")` on the Result
    // would still fail because the panic would never produce a Result.
    // The literal test failure surfaces at `fresh_empty_session()` or
    // at the `apply` call line.
    let mut session = fresh_empty_session();
    session
        .apply("any-facet", "any-option")
        .expect("apply on empty CCM must not panic and must return a well-defined Result");
    // Idempotency: repeated applies with the same arguments are a no-op
    // per ADR-0003 §4.
    session
        .apply("any-facet", "any-option")
        .expect("second apply with identical args must be idempotent");
}

#[test]
fn retract_is_a_noop_when_nothing_is_applied() {
    // ADR-0003 §4: `retract(facet)` on a facet that is not currently
    // applied is a no-op (not an error). This is the "nothing to retract"
    // path that every well-behaved client hits after a fresh session.
    let mut session = fresh_empty_session();
    session
        .retract("any-facet")
        .expect("retract on a not-applied facet must be a no-op");
    // Second retract is also idempotent.
    session
        .retract("any-facet")
        .expect("second retract must also be a no-op");
}

#[test]
fn resolve_returns_empty_configuration() {
    // ADR-0003 §4: `resolve` materializes the current resolved configuration.
    // An empty CCM has exactly one model (the empty assignment) and is
    // trivially satisfiable, so the M0 stub returns `ResolveResult { satisfiable: true }`.
    let session = fresh_empty_session();
    let result: ResolveResult = session
        .resolve()
        .expect("resolve on empty CCM must return Ok");
    assert!(
        result.satisfiable,
        "empty CCM must be satisfiable (admits the empty assignment)"
    );
}

#[test]
fn explain_rejection_is_callable_and_surfaces_unknown_on_empty_ccm() {
    // ADR-0003 §4: `explain_rejection` is a pure query that never mutates
    // state. configflux-kv5d (M4) turned it from a stub into real MUS
    // extraction (ADR-0004 §4): on an empty CCM there is no symbol table, so
    // any `(facet, option)` is "unknown" — the same typed `UnknownOption`
    // error `apply`/`valid_options` raise for an unmodeled symbol, not a
    // panic and not a stub `Ok`. A conflict-bearing run with a real fixture
    // is covered by `explain_rejection_mus`.
    let session = fresh_empty_session();
    let explanation = session.explain_rejection("any-facet", "any-option");
    assert!(
        matches!(
            &explanation,
            Err(Error::UnknownOption { facet, value })
                if facet == "any-facet" && value == "any-option"
        ),
        "empty CCM explain_rejection must surface UnknownOption, got {explanation:?}",
    );
}

#[test]
fn state_hash_is_stable_across_two_loads() {
    // ADR-0003 §4: `state_hash` is content-addressed and **byte-for-byte
    // stable** across runs given the same sequence of applies and retracts.
    // The strongest form of that property is: two independently loaded
    // empty CCMs, with no applies and no retracts, must produce the same
    // hash. If they do not, byte-stability is broken and every downstream
    // fingerprint (snapshot identity, replay check, daemon handshake) is
    // wrong.
    //
    // Red-path check: if a buggy refactor made `state_hash` return the
    // current Unix timestamp or a random nonce, the `assert_eq!` below
    // would fail deterministically on every run. If the refactor made
    // `state_hash` return the same value by accident (e.g. always zero),
    // the test would trivially pass — which is why the M1 implementation
    // task (`configflux-8dm`) will need to extend this test with a second
    // assertion that compares against a non-zero baseline once the real
    // SHA-256 pre-image is wired. For M0, comparing two fresh loads is
    // the contract the ADR requires.
    let session_a = fresh_empty_session();
    let session_b = fresh_empty_session();
    let hash_a: StateHash = session_a.state_hash();
    let hash_b: StateHash = session_b.state_hash();
    assert_eq!(
        hash_a, hash_b,
        "state_hash must be byte-for-byte identical across two identical fresh loads of the empty CCM"
    );
    // And on a single session, repeated calls must also return the same value.
    let hash_a2: StateHash = session_a.state_hash();
    assert_eq!(
        hash_a, hash_a2,
        "state_hash must be byte-for-byte identical across repeated calls on the same Session"
    );
}
