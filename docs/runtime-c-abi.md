# Runtime C ABI (v1.3)

Status: Stable (v1.3)
Date: 2026-02-15 (stabilized 2026-07-23)

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

### 3.1 Threading

A session handle is **single-threaded**: it must not be used concurrently from
more than one thread. In particular, `configflux_runtime_session_execute_json`
must not be called concurrently or reentrantly on the same handle — each call
mutates the retained session snapshot in place, so overlapping execute calls on
one handle are undefined. Distinct handles are independent and may be used from
distinct threads without coordination.

## 4. Session Model

1. `session_open` takes a serialized `RuntimeOpenRequest` JSON payload.
2. Runtime state (`runtime_snapshot`) is retained in the opaque session.
3. `session_execute_json` takes operation code + request JSON (without `runtime_snapshot`).
4. The ABI injects the current snapshot, executes runtime-core API, returns response JSON, and
   updates session snapshot if response contains `runtime_snapshot`.
5. `configflux_runtime_session_snapshot_json(handle, out_json)` exports the session's
   current `runtime_snapshot` as a heap-owned JSON C string (release with
   `configflux_runtime_string_free`). It is a read-only accessor and does not mutate
   session state.

### 4.1 Open-time solver-model precondition (since v1.1)

`session_open` enforces the same `.ccm` solver-model precondition the runtime CLI
`runtime-open` enforces (ADR-0030 D2): the snapshot's `ccm_ref` must resolve to a usable
solver model (a loadable artifact with a populated symbol table) that is bound to the
snapshot's own `model_hash`. When it does not — an empty reference, an unloadable
artifact, a symbol-less stub, or an artifact bound to a different model — the open fails
closed:
the response JSON carries `status=error` with diagnostic code
`E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`, and **no** session handle is returned
(`out_handle` stays null). The boundary status (Section 6) is still `Ok` because the call
itself was well-formed; the refusal is a runtime-domain rejection in the envelope. This
makes CLI-driven and SDK-driven (C++/ROS2) opens behave identically — there is no open
path that accepts a snapshot lacking a loadable `.ccm`.

### 4.2 Closed-facet domains on the open payload (since v1.2)

The exported symbols and their C signatures are unchanged from v1.1. Three
behaviours are new, and the minor is the only channel that advertises them:

1. `session_open` honours an optional `closed_facet_domains` key on the open
   request JSON — an object of the form `{"<facet>": ["<value>", ...]}` carrying
   the declared values of every **closed** facet, copied from the resolve
   result's field of the same name. Open facets are absent rather than flagged.
   The runtime uses it so a rejection whose explanation mentions a closed facet
   only negatively still names the constraint that was violated, instead of
   reporting the model as over-constrained. Omitting the key is valid and
   preserves v1.1 behaviour exactly; the open still succeeds and the message
   quality degrades.
2. `session_snapshot_json` output gains the same key. It is not
   omitted-when-empty (no snapshot field is), so an unpopulated table appears as
   `"closed_facet_domains":{}`. A client that parses the snapshot strictly must
   tolerate it.
3. A **supplied** table is validated against the bound solver model. If any
   `{facet}.{value}` pair is absent from that model's symbol table, the open
   fails closed exactly as the Section 4.1 precondition does: response JSON with
   `status=error` and diagnostic code `E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN`, no
   session handle, boundary status still `Ok`. This is a new way for an open to
   fail, which is why the minor moves. The check verifies that each supplied
   facet and value exists in the bound model; it cannot verify that a facet is
   closed, because the symbol table carries no cardinality.

The handshake rule is unchanged, so `expected_minor = 0` and `expected_minor = 1`
clients keep passing against ABI 1.2.

### 4.3 Declared facet bindings (since v1.3)

The exported symbols and their C signatures are unchanged from v1.2. Two
behaviours are new, and the minor is the only channel that advertises them:

1. `session_snapshot_json` output may carry a `facet` key inside a resolved
   parameter, naming the facet that parameter is the model's declared runtime
   handle for. The key is omitted for a parameter that declares no binding, so
   every payload produced by a model without bindings is byte-identical to
   v1.2. A client that parses the snapshot strictly must tolerate the key.
2. Constraint enforcement on the write path follows those DECLARED bindings.
   A write to a parameter that declares `facet: <name>` is evaluated against
   the model's constraints for `<name>`; a write to a parameter that merely
   shares a facet's name is **not** enforced, and keeps its type, limit and
   lifecycle checks unchanged. Before v1.3 the runtime inferred the facet from
   the parameter path's last segment, so a same-named parameter was enforced
   as if it were the facet's handle.

The handshake rule is unchanged, so `expected_minor` 0, 1 and 2 clients keep
passing against ABI 1.3.

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
