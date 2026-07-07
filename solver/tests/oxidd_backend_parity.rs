// SPDX-License-Identifier: BUSL-1.1
//
// OxiddBackend parity integration test — configflux-8dm.1
//
// This file re-exercises the public `solver::Session` API against
// `OxiddBackend`, which wraps oxidd 0.11.0 behind the `SolverBackend`
// trait per ADR-0003 §3 and docs/security/oxidd-audit.md §6.
//
// Three purposes:
//
// 1. Prove `OxiddBackend` satisfies `SolverBackend` at compile time.
//    (`Session::<OxiddBackend>::...` in an external-crate integration
//    test surfaces a trait-bound failure as a compile error, not a
//    runtime panic.)
// 2. Prove the public Session API does not leak oxidd types. This file
//    never imports `oxidd::*`; if a Session method signature ever
//    surfaced an oxidd type, this file would stop compiling.
// 3. Prove NullBackend semantics are preserved through the trait
//    boundary. An empty Session over OxiddBackend must behave
//    identically to an empty Session over NullBackend on the
//    round-trip-empty contract (configflux-6tl).

use std::path::Path;

use solver::{Error, OxiddBackend, ResolveResult, Session, SolverBackend, ValidOptions};

/// Path passed to `Session::load_ccm`. The M0 `Ccm::load_from_cmp` stub
/// ignores its argument; downstream M1 tasks will replace this with a
/// real `.cmp` path.
const UNUSED_CCM_PATH: &str = "unused/oxidd_backend_parity.ccm";

fn fresh_empty_oxidd_session() -> Session<OxiddBackend> {
    let ccm = Session::<OxiddBackend>::load_ccm(Path::new(UNUSED_CCM_PATH))
        .expect("load_ccm on empty CCM must succeed per ADR-0003 §4");
    Session::<OxiddBackend>::new(ccm)
        .expect("Session::new on empty CCM must succeed per ADR-0003 §4")
}

#[test]
fn oxidd_backend_session_round_trip_matches_null_backend() {
    // Mirrors the ADR-0003 §4 call sequence in `round_trip_empty.rs`
    // but over OxiddBackend. A regression in the wrapper surfaces here
    // as a shape mismatch, not a panic.
    let mut session = fresh_empty_oxidd_session();

    let options: ValidOptions = session
        .valid_options("any-facet")
        .expect("valid_options on oxidd-backed empty session must return Ok");
    assert_eq!(options.count, 0);

    session
        .apply("engine", "v6")
        .expect("apply on oxidd-backed empty session must return Ok");
    session
        .retract("engine")
        .expect("retract on oxidd-backed empty session must return Ok");

    // configflux-kv5d: real MUS extraction (ADR-0004 §4). On the oxidd-backed
    // empty session there is no symbol table, so the candidate is "unknown" —
    // a typed `UnknownOption`, not the old stub `Ok`.
    let explanation = session.explain_rejection("engine", "v6");
    assert!(
        matches!(&explanation, Err(Error::UnknownOption { facet, value }) if facet == "engine" && value == "v6"),
        "explain_rejection on the empty oxidd session must be UnknownOption, got {explanation:?}",
    );

    let resolved: ResolveResult = session
        .resolve()
        .expect("resolve on oxidd-backed empty session must return Ok");
    assert!(resolved.satisfiable);
}

#[test]
fn oxidd_backend_advertises_stable_backend_id() {
    // ADR-0005 §2 pins `ccm.manifest.json.algorithm` as the stable
    // backend identifier. Pinning the string here ensures the CCM
    // manifest writer (landing in configflux-8dm.2+) sees the expected
    // tag; a silent oxidd bump that changed this value would fail
    // here before shipping.
    let session = fresh_empty_oxidd_session();
    assert_eq!(
        session.backend().backend_id(),
        "oxidd-v0.11",
        "oxidd backend must advertise a stable backend_id tag"
    );
}

#[test]
fn oxidd_backend_state_hash_is_stable_across_loads() {
    // ADR-0003 §4 makes `state_hash` a backend-agnostic byte-stability
    // property. Two fresh empty sessions — across the trait boundary —
    // must yield the same hash.
    let session_a = fresh_empty_oxidd_session();
    let session_b = fresh_empty_oxidd_session();
    assert_eq!(
        session_a.state_hash(),
        session_b.state_hash(),
        "state_hash must be byte-for-byte identical across two identical fresh loads of the oxidd-backed empty CCM"
    );
}
