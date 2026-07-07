# Runtime v2 Operations Runbook (Operator + SDK)

Status: active  
Date: 2026-02-18  

## 1. Purpose

This runbook defines day-2 operating procedures for Runtime v2:

1. commissioning with direct push (offline-capable),
2. backend sync mode operation,
3. telemetry/audit sink handling,
4. deterministic failure triage,
5. C++ and ROS2 SDK integration checkpoints.

Use this document together with:

1. `docs/runtime-v2-contract.md`
2. `docs/runtime-v2-sync-transport.md`
3. `docs/runtime-v2-security-envelope.md`

## 2. Operating Modes

Runtime v2 supports two primary apply paths:

1. Direct push commissioning (`source = direct_push`, `backend_connected = false`)
2. Backend sync apply (`source = backend`, `backend_connected = true`)

Both paths must preserve:

1. deterministic diagnostics and exit behavior,
2. hash-lineage continuity (`model_hash`, `resolve_hash`, configuration IDs),
3. audit event generation and replayability.

## 3. Direct-Push Commissioning (Offline-Capable)

### 3.1 Preconditions

1. Runtime snapshot opened successfully with `runtime-open`.
2. Technician payload already converted to path/value writes.
3. Delta preconditions available (`base_configuration_id`, `before_leaf_hash` for changed paths).

### 3.2 Commissioning Apply

Run:

```bash
bazel run //runtime:runtime -- pull-updates --request-file /tmp/direct_push.request.json --response-file /tmp/direct_push.response.json
```

`/tmp/direct_push.request.json` must include:

1. `schema_version` (use the current product schema expected by the runtime binary),
2. `runtime_snapshot`,
3. `actor` and optional `reason`,
4. `backend_connected = false`,
5. `source = "direct_push"`,
6. `writes[]` with `path`, `value`, `before_leaf_hash`, `after_leaf_hash`,
7. `base_configuration_id`,
8. `target_configuration_id`,
9. `full_snapshot` (`false` for normal delta-first commissioning).

Expected response checks:

1. `status = ok`,
2. `applied_paths[]` contains all changed canonical paths,
3. `audit_event_id` is present.

### 3.3 Reconciliation Export

When backend is unavailable, export pending sync/audit material:

```bash
bazel run //runtime:runtime -- export-pending-sync-bundle --request-file /tmp/export_pending.request.json --response-file /tmp/export_pending.response.json
```

Expected response checks:

1. `bundle.bundle_id` is present,
2. `bundle.pending_audit_count >= 1` after direct push,
3. `bundle.pending_audit_events[]` contains direct-push lineage events.

Reference verification coverage:

1. `REQ-RUN-030` (`//runtime:runtime_v2_direct_push_gate_test`)
2. `REQ-RUN-036` (`//runtime:runtime_v2_audit_replay_gate_test`)

## 4. Backend Sync Mode

### 4.1 Normal Cycle

1. Check for updates:
   - `check-for-updates`
2. Pull/apply updates:
   - `pull-updates` with `source = backend`, `backend_connected = true`
3. Verify sync state:
   - `get-sync-status`
4. Upload audit queue:
   - `push-audit-events` (online mode drains pending queue)

### 4.2 Conflict and Delta Preconditions

Runtime enforces:

1. `base_configuration_id` precondition checks,
2. per-path `before_leaf_hash` checks,
3. upstream-authoritative apply for changed keys,
4. unchanged-path preservation outside `changed_paths`.

Reference verification coverage:

1. `REQ-RUN-029` (`//runtime:runtime_v2_sync_delta_gate_test`)
2. `REQ-RUN-026` (`//runtime:runtime_cli_test`, sync command surface)

## 5. Telemetry and Audit Sink Operations

Telemetry and audit share the same offline-first operational posture:

1. online: push/flush via transport,
2. offline: retain durable pending state,
3. reconnect: replay deferred state deterministically.

Operational checks:

1. `push-audit-events` with `backend_connected = false`:
   - expect `pushed_count = 0`
   - expect `pending_count > 0` if events exist
2. `push-audit-events` with `backend_connected = true`:
   - expect monotonic `last_uploaded_sequence`
   - expect eventual `pending_count = 0` after drains
3. `export-pending-sync-bundle`:
   - verify pending state can be handed to reconciliation tooling

Reference verification coverage:

1. `REQ-RUN-027` (`//runtime:runtime_cli_test`)
2. `REQ-RUN-036` (`//runtime:runtime_v2_audit_replay_gate_test`)

## 6. Failure Triage Runbook

### 6.1 First Response

1. Capture request and response JSON payloads.
2. Record Bazel invocation ID and command used.
3. Preserve runtime snapshot if reproducibility requires replay.

### 6.2 Diagnostic Family Routing

1. `E_RUNTIME_CLI_*`: input/transport/I/O misuse (`request-file`, malformed JSON, size bounds).
2. `E_RUNTIME_SYNC_*`: backend connectivity, delta preconditions, or conflict handling.
3. `E_RUNTIME_SECURITY_*`: authn/authz/TLS/security envelope failures (fail closed).
4. `E_RUNTIME_PERSIST_*`: persistence checksum/recovery/format issues.
5. `E_RUNTIME_EVENT_INVALID` or `E_RUNTIME_AUDIT_INVALID`: malformed event/audit state.

### 6.3 Required Escalation Process

1. Open an issue for unresolved defects.
2. Attach:
   - minimal reproducer payloads,
   - invocation IDs,
   - observed diagnostics.

## 7. SDK Integration Guidance

### 7.1 C++ SDK (runtime C ABI wrapper)

1. Follow ABI handshake + session ownership rules in `docs/runtime-c-abi.md`.
2. Use `RuntimeSession` (`sdk/cpp/include/configflux/sdk/runtime_session.h`) as the default integration boundary.
3. Validate with:
   - `bazel test //sdk/cpp:runtime_sdk_core_test`
   - `bazel test //sdk/cpp:runtime_sdk_core_linked_abi_test`

Relevant requirements:

1. `REQ-SDK-CPP-001` .. `REQ-SDK-CPP-004`
2. `REQ-REL-004`

### 7.2 ROS2 SDK (ament_cmake + colcon via Bazel)

1. Keep implementation under `sdk/ros2` and outside `runtime/`.
2. Validate adapter behavior with:
   - `bazel test //sdk/ros2:runtime_parameter_adapter_test`
   - `bazel test //sdk/ros2:runtime_control_adapter_test`
   - `bazel test //sdk/ros2:runtime_diagnostics_bridge_test`
3. Validate package-level build with Bazel wrapper:

```bash
bazel run //tools:run_ros2_colcon -- --workspace /workspaces/configflux/sdk/ros2 -- --packages-select configflux_ros2_sdk
```

Relevant requirements:

1. `REQ-SDK-ROS2-001` .. `REQ-SDK-ROS2-004`
2. `REQ-REL-005` .. `REQ-REL-008`

## 8. References

1. `docs/runtime-v2-contract.md`
2. `docs/runtime-v2-state-machine.md`
3. `docs/runtime-v2-persistence.md`
4. `docs/runtime-v2-sync-transport.md`
5. `docs/runtime-v2-security-envelope.md`
6. `docs/runtime-c-abi.md`
7. `sdk/cpp/README.md`
8. `sdk/ros2/README.md`
