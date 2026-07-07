# Runtime C ABI (v1.1)

Status: draft
Date: 2026-02-15

## 1. Scope

This document defines the first stable C ABI surface for ConfigFlux runtime-core operations.
The ABI is implemented in Rust (`runtime/src/runtime_c_abi.rs`) with no first-party `*.c`
sources. It lives in the runtime crate so the open entrypoint can enforce the
solver-model precondition described in Section 4 (the precondition requires the solver
engine, which the compiler core does not depend on; see ADR-0003 and ADR-0030).

## 2. Version Handshake

Call `configflux_runtime_abi_handshake(expected_major, expected_minor, out_version)` before
session usage.

Compatibility rule:
1. `expected_major` must equal ABI major.
2. `expected_minor` must be `<=` ABI minor.

## 3. Ownership Rules

1. Session handles are opaque (`ConfigFluxRuntimeSessionHandle*`).
2. Create with `configflux_runtime_session_open`.
3. Destroy with `configflux_runtime_session_close`.
4. Any returned JSON C string is heap-owned by the ABI and must be released with
   `configflux_runtime_string_free`.

## 4. Session Model

1. `session_open` takes a serialized `RuntimeOpenRequest` JSON payload.
2. Runtime state (`runtime_snapshot`) is retained in the opaque session.
3. `session_execute_json` takes operation code + request JSON (without `runtime_snapshot`).
4. The ABI injects the current snapshot, executes runtime-core API, returns response JSON, and
   updates session snapshot if response contains `runtime_snapshot`.

### 4.1 Open-time solver-model precondition (since v1.1)

`session_open` enforces the same `.ccm` solver-model precondition the runtime CLI
`runtime-open` enforces (ADR-0030 D2): the snapshot's `ccm_ref` must resolve to a usable
solver model (a loadable artifact with a populated symbol table). When it does not — an
empty reference, an unloadable artifact, or a symbol-less stub — the open fails closed:
the response JSON carries `status=error` with diagnostic code
`E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`, and **no** session handle is returned
(`out_handle` stays null). The boundary status (Section 6) is still `Ok` because the call
itself was well-formed; the refusal is a runtime-domain rejection in the envelope. This
makes CLI-driven and SDK-driven (C++/ROS2) opens behave identically — there is no open
path that accepts a snapshot lacking a loadable `.ccm`.

## 5. Operation Codes

`ConfigFluxRuntimeOperation`:
1. `GetScopeMetadata` (1)
2. `ListParameters` (2)
3. `GetParameter` (3)
4. `SetParameter` (4)
5. `SetParametersAtomically` (5)
6. `ListDirtyParameters` (6)
7. `GetDirtyMetadata` (7)
8. `RollbackDirty` (8)
9. `CommitConfiguration` (9)
10. `GetConfigurationIdentity` (10)
11. `SetAutoResetPolicy` (11)
12. `GetAutoResetPolicy` (12)
13. `CheckForUpdates` (13)
14. `PullUpdates` (14)
15. `GetSyncStatus` (15)
16. `SubscribeEvents` (16)
17. `PushAuditEvents` (17)
18. `ExportPendingSyncBundle` (18)

## 6. ABI Status Mapping

`ConfigFluxRuntimeAbiStatus` provides deterministic boundary failures:
1. `Ok`
2. `NullPointer`
3. `InvalidUtf8`
4. `InvalidJson`
5. `VersionMismatch`
6. `InvalidHandle`
7. `UnsupportedOperation`
8. `InternalError`
9. `CStringContainsNul`

Runtime domain rejections remain in the returned JSON envelope (`status=error` + diagnostics).

## 7. SDK Placement Rule

Wrapper/SDK implementation remains outside `runtime/`:
1. `sdk/cpp`
2. `sdk/ros2`

The C ABI boundary is allowed, but first-party wrapper code remains Rust/C++ only.
