// SPDX-License-Identifier: BUSL-1.1
//
// ConfigFlux solver crate. Apache-2.0 licensed, as is the rest of the
// repository per ADR-0029; `solver/` was Apache-2.0 from its first commit
// (originally the ADR-0007 §1 carve-out) and the relicense made that
// uniform. Every file under `solver/` carries the Apache-2.0 SPDX header.
//
// # Crate purpose
//
// This crate is the runtime configuration solver. Given a compiled
// constraint model (`.ccm`) artifact emitted by the compiler, it
// holds a `Session` that answers queries about which options remain
// valid, applies and retracts selections, explains rejections, and
// resolves the current configuration. The public `Session` API is the
// sole entry point the rest of the workspace calls through; interpreter
// and runtime hold a `Session` in-process per ADR-0003 §5.
//
// # Dependency direction
//
// Per ADR-0003 §2:
//
// ```text
// compiler  →  (emits .ccm on disk)  →  solver  ←  interpreter, runtime
// ```
//
// - `compiler` never imports `solver`.
// - `solver` never imports `compiler`.
// - `solver` never parses `.cmp` chunks; its sole input is the `.ccm`
//   artifact format defined by ADR-0005.
// - `interpreter` and `runtime` depend on `solver` and hold a
//   `solver::Session` per process.
//
// # Scaffolding vs. M1
//
// This is the M0 scaffolding crate produced by `configflux-8vb`. The
// public API shape is pinned by ADR-0003 §4 and is the same shape the
// M1 task (`configflux-8dm`) will fill in. In M0 every `Session` method
// is a stub that returns an `Ok` with a placeholder value so that a
// round-trip smoke test can call each method without panicking. No
// BDD operations, no `.ccm` parsing, no `oxidd` dependency. Adding
// those is explicitly M1's job per the task brief for `configflux-8vb`.

#![deny(missing_debug_implementations)]

pub mod backend;
// `backend_oxidd` is the single module permitted to `use oxidd::*` per
// ADR-0003 §3 and `docs/security/oxidd-audit.md` §5–§6. Every other
// file in this crate — including `session` and the Null backend —
// calls through the `SolverBackend` trait in `backend`. Wired in by
// configflux-8dm.1.
pub mod backend_oxidd;
// `backend_cudd` is the parallel single module permitted to
// `use cudd_sys::*` per ADR-0003 §3 and the ADR-0004 second-pass
// amendment 2026-05-09 (configflux-dwwv). Lives alongside
// `backend_oxidd` — the oxidd backend remains the reference for the
// existing 10k+0/0 fixture; the CUDD backend is the M1 mitigation
// rung 2 path. Both implement the same `SolverBackend` trait and
// must be semantically interchangeable on the ADR-0005 §4 canonical
// node table. Wired in by configflux-jxk6.
pub mod backend_cudd;
pub mod ccm;
// `explain` carries `Session::explain_rejection()` and the solver-owned
// labeled-MUS types (ADR-0004 §4) — split out of `session.rs` as a sibling
// `impl Session<B>` block to keep that file under the line cap (the same
// shape `resolve` follows). It calls through the `SatBackend` trait only;
// no `batsat::*` import lives outside `sat_backend_batsat.rs`. Wired in by
// configflux-kv5d.
pub mod explain;
// `ccm_format` is private to the crate per ADR-0005 §1 — the on-disk
// layout is an implementation detail; public consumers (interpreter,
// runtime) go through `Ccm` and `Session`. Wired in by configflux-8dm.2.
pub(crate) mod ccm_format;
// `ccm_multi_part` is the v2 multi-part loader per ADR-0005 Amendment 1
// §11–§16 and ADR-0012 §4–§5. Crate-private: the multi-partition fan-out
// (PartitionCcm, MultiPartCcm) is implementation detail behind the
// existing public `Ccm` and `Session` surfaces, per the bd-configflux-mwyp
// hard constraint and ADR-0003 §1. Wired in by configflux-mwyp.
pub(crate) mod ccm_multi_part;
// `partition_manifest` reads the v2 `partition-manifest.json` file per
// ADR-0005 Amendment 1 §13. Crate-private for the same ADR-0003 §1
// reason as `ccm_multi_part`.
pub(crate) mod partition_manifest;
// `partition_session` carries the multi-partition fan-out for the
// public `Session<B>` API per ADR-0012 §1, §6, §7. Crate-private:
// `PartitionSession<B>`, `PartitionFormulaHandle`, and `UndoEntry` are
// implementation detail behind the unchanged `Session::valid_options`
// / `apply` / `retract` / `state_hash` signatures (the bd-0r62 hard
// ADR-0003 constraint). Wired in by configflux-0r62.
pub(crate) mod partition_session;
// `cudd_translate` is the internal CCM-bytestream <-> CUDD translation
// pass per ADR-0005 §4 (configflux-fvew). Crate-private: the byte
// layout is implementation detail per ADR-0005 §1, and the CUDD-side
// pointers are the same `*mut DdNode` types `backend_cudd.rs` is the
// sole authorized importer of (ADR-0003 §3 / ADR-0004 amendment).
// Exposed only to `backend_cudd` and to crate-internal integration
// surfaces; never re-exported.
pub(crate) mod cudd_translate;
// `resolve` carries `Session::resolve()` and `ResolveResult` — split out
// of `session.rs` to keep that file under the line cap (configflux-i0ne).
// It is an `impl Session<B>` block plus the public `ResolveResult` type.
pub mod resolve;
// `sat_backend` is the narrow CDCL boundary trait for deletion-based MUS
// extraction per ADR-0004 §4 — the SAT analogue of `SolverBackend`. It
// does NOT import `batsat`; the one file that does is
// `solver/src/sat_backend_batsat.rs`. Wired in by configflux-tcjw. No
// caller exists yet — `Session::explain_rejection` is configflux-kv5d.
pub mod sat_backend;
// `sat_backend_batsat` is the single module permitted to `use batsat::*`
// per ADR-0004 §4 (the AC-5 mitigation leg from
// docs/security/dep-audit-v0.4.0.md §2). Every other file in this crate
// calls through the `SatBackend` trait in `sat_backend`. Wired in by
// configflux-tcjw.
pub mod sat_backend_batsat;
pub mod session;
pub mod snapshot;

pub use backend::{
    BackendError, CapacityHints, FormulaHandle, NullBackend, SolverBackend, VarHandle,
    VariableOrder,
};
pub use backend_cudd::CuddBackend;
pub use backend_oxidd::OxiddBackend;
pub use ccm::{Bdd, Ccm, CcmError, Symbols, CCM_SCHEMA_VERSION_V1};
// `BddNode` is public because it appears in `SolverBackend::deserialize_bdd`'s
// signature; external backends (a future BuDDy wrapper) need it to
// implement the trait. It is intentionally `pub` but the on-disk layout
// of `ccm.bdd.bin` is not — `BddNode` is the decoded runtime shape.
pub use ccm_format::BddNode;
pub use resolve::ResolveResult;
// SAT boundary (ADR-0004 §4) — exported so a future `Session::explain_rejection`
// (configflux-kv5d) and any alternative CDCL backend can name the trait and
// its opaque handle types. No batsat type is re-exported here; the concrete
// batsat wrapper stays behind `BatsatBackend`.
pub use sat_backend::{SatBackend, SatError, SatLit, SatOutcome, SatVar};
pub use sat_backend_batsat::BatsatBackend;
// `LabeledCore` and friends are the solver-owned labeled-MUS types from
// `explain_rejection` (configflux-kv5d). Re-exported so the interpreter and
// runtime wrappers (configflux-whyt / configflux-3b5y) can name them when
// converting into the compiler-side `UnsatCore`. Labeled strings only — no
// compiler type and no raw BDD/batsat index ever appears in them (ADR-0003
// §2 / ADR-0031 D3).
pub use session::{
    CoreConstraintKind, Error, LabeledAtom, LabeledConstraint, LabeledCore, RejectionExplanation,
    Session, StateHash, ValidOptions,
};
pub use snapshot::{Snapshot, SNAPSHOT_SCHEMA_VERSION_V1};

#[cfg(test)]
mod smoke_tests {
    //! Compile-link smoke test for the scaffolding crate.
    //!
    //! This is deliberately a smaller, lighter version of the full
    //! round-trip-empty test tracked in `configflux-6tl` (which is the
    //! M0 exit criterion and is blocked on this task). Its job is only
    //! to prove that the crate compiles, that every `Session` method
    //! from ADR-0003 §4 is callable through the public API, and that
    //! none of the M0 stubs panic or return `Err` on a fresh empty
    //! session. `configflux-6tl` extends this into a real integration
    //! test with a hand-authored CCM fixture once M1 wires the real
    //! loader.
    use super::*;
    use std::path::Path;

    #[test]
    fn empty_session_round_trip_smoke() {
        // 1. Load a `.ccm` (M0 stub: empty handle for any path).
        let ccm = Session::<NullBackend>::load_ccm(Path::new("unused/path.ccm"))
            .expect("M0 stub never fails on load_ccm");
        assert_eq!(ccm.ccm_hash(), [0u8; 32]);
        assert_eq!(ccm.schema_version(), CCM_SCHEMA_VERSION_V1);

        // 2. Construct a Session over NullBackend.
        let mut session =
            Session::<NullBackend>::new(ccm).expect("M0 stub never fails on Session::new");
        assert_eq!(session.backend().backend_id(), "null-v0");

        // 3. Exercise every public Session method from ADR-0003 §4.
        //    Every call must return Ok (or a non-panicking StateHash)
        //    per the task brief's "stubs return dummy Ok values (not
        //    panics) so a round-trip test can call each" acceptance
        //    criterion.
        let options = session
            .valid_options("any-facet")
            .expect("valid_options stub returns Ok");
        assert_eq!(options.count, 0);

        session
            .apply("engine", "v6")
            .expect("apply stub returns Ok");

        session
            .retract("engine")
            .expect("retract stub returns Ok");

        // configflux-kv5d turned `explain_rejection` from a stub into real
        // MUS extraction (ADR-0004 §4). On the empty-Ccm session there is no
        // symbol table, so an `(facet, option)` pair is "unknown" — the same
        // typed error `apply`/`valid_options` raise for an unmodeled symbol,
        // not a panic. A real conflict-explaining run is exercised by the
        // dedicated `explain_rejection_mus` integration test.
        let explanation = session.explain_rejection("engine", "v6");
        assert!(
            matches!(
                &explanation,
                Err(Error::UnknownOption { facet, value }) if facet == "engine" && value == "v6"
            ),
            "explain_rejection on the empty-Ccm session must surface UnknownOption, got {explanation:?}",
        );

        let resolved = session.resolve().expect("resolve stub returns Ok");
        assert!(resolved.satisfiable);

        let hash = session.state_hash();
        assert_eq!(hash, StateHash::zero());

        // 4. Idempotency spot-check: second apply/retract must still be Ok.
        session
            .apply("engine", "v6")
            .expect("second apply is idempotent");
        session
            .apply("engine", "v6")
            .expect("third apply is idempotent");
        session
            .retract("engine")
            .expect("second retract is idempotent");
    }

    #[test]
    fn public_reexports_are_visible() {
        // Sanity-check that every type ADR-0003 §4 names is reachable
        // from the crate root. Without these re-exports, the smoke test
        // above compiles but downstream consumers (interpreter, runtime,
        // M1 `configflux-8dm`) would need to import from internal modules,
        // which would make the public API harder to refactor later.
        let _: Snapshot = Snapshot::empty();
        let _: Ccm = Ccm::empty();
        let _: StateHash = StateHash::zero();
        let _: ValidOptions = ValidOptions::default();
        let _: ResolveResult = ResolveResult::default();
        let _: RejectionExplanation = RejectionExplanation::default();
        let _: VariableOrder = VariableOrder::empty();
        let _: CapacityHints = CapacityHints::default();
        let _err: CcmError = CcmError::ManifestParse;
        let _berr: BackendError = BackendError::OutOfCapacity;
        let _vh: VarHandle = VarHandle(0);
        let _fh: FormulaHandle = FormulaHandle(0);
    }
}
