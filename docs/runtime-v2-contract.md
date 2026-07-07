# Runtime API and CLI Contract v2 (Frozen)

Status: frozen
Date: 2026-02-14
Supersedes: `docs/runtime-cli-contract.md` for v2 operations

## 1. Scope

This document freezes Runtime v2 request/response contracts for:

1. Dirty metadata operations
2. Full-state commit and rollback
3. Auto-reset policy control
4. Sync status and update pull/apply
5. Event subscription and deterministic event replay

Runtime v1 commands remain supported for backward compatibility.

## 2. Versioning Rules

1. `schema_version = 2` identifies Runtime v2 envelopes.
2. `schema_version = 1` remains valid for v1 command envelopes.
3. Runtime responses always echo the requested schema version.
4. Unknown schema versions fail with `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION`.
5. Runtime-core semantics are defined by this document; CLI transport behavior remains deterministic and follows v1 transport invariants.

## 3. Deterministic Transport and Exit Codes

Transport behavior remains byte-deterministic for equivalent request bytes.

Exit code mapping is unchanged:

1. `0`: response `status = ok`
2. `2`: response `status = error` (domain validation/runtime rejection)
3. `1`: CLI/transport failure (`args`, I/O, malformed request, oversized request)

## 4. Common Envelope Fields

All Runtime v2 responses include:

1. `schema_version`
2. `status` (`ok` or `error`)
3. `model_hash`
4. `resolve_hash`
5. `committed_configuration_id`
6. `working_configuration_id`
7. `event_sequence`
8. `error_count`
9. `warning_count`
10. `diagnostics[]`

Determinism rules:

1. `diagnostics[]` ordering is stable and deterministic.
2. Path lists are sorted in canonical lexical order.
3. Event sequence IDs are monotonic per runtime instance and persisted across restart.

## 5. Runtime v2 Operation Set

Runtime v1 operations remain:

1. `runtime_open`
2. `get_scope_metadata`
3. `list_parameters`
4. `get_parameter`
5. `set_parameter`

Runtime v2 adds:

1. `set_parameters_atomically`
2. `list_dirty_parameters`
3. `get_dirty_metadata`
4. `rollback_dirty`
5. `commit_configuration`
6. `get_configuration_identity`
7. `set_auto_reset_policy`
8. `get_auto_reset_policy`
9. `check_for_updates`
10. `pull_updates`
11. `get_sync_status`
12. `push_audit_events`
13. `subscribe_events`
14. `export_pending_sync_bundle`

## 6. Operation Contracts (v2 Additions)

### 6.1 `set_parameters_atomically`

Request (required):

1. `runtime_snapshot`
2. `writes[]`: `{ path, value, actor, reason? }`
3. `expected_working_configuration_id?`

Response (required):

1. `applied_count`
2. `rejected_paths[]` with deterministic diagnostics
3. `dirty_generation_max`

Semantics:

1. All writes pass or all fail.
2. On success, dirty metadata is generated for each changed path.

### 6.2 `list_dirty_parameters`

Request:

1. `runtime_snapshot`
2. `scope`

Response:

1. `dirty_paths[]` (sorted)
2. `dirty_count`

### 6.3 `get_dirty_metadata`

Request:

1. `runtime_snapshot`
2. `scope`
3. `path`

Response:

1. `dirty` (`true|false`)
2. `dirty_since_utc`
3. `dirty_actor`
4. `dirty_reason`
5. `reset_deadline_utc`
6. `dirty_generation`

### 6.4 `rollback_dirty`

Request:

1. `runtime_snapshot`
2. `mode`: `all | subset`
3. `paths[]` (required when `mode = subset`)
4. `actor`
5. `reason?`

Response:

1. `rolled_back_paths[]`
2. `remaining_dirty_paths[]`
3. `rollback_event_id`

### 6.5 `commit_configuration`

Request:

1. `runtime_snapshot`
2. `actor`
3. `reason?`
4. `expected_base_configuration_id?`
5. `changed_paths_hint?[]`

Response:

1. `commit_id`
2. `base_configuration_id`
3. `target_configuration_id`
4. `changed_paths[]` with `before_leaf_hash` and `after_leaf_hash`
5. `delta_manifest`

Semantics:

1. Commit remains full-state semantic at API level.
2. Persistence and sync artifacts are delta-first and include path-granular change manifests.

### 6.6 `get_configuration_identity`

Request:

1. `runtime_snapshot`

Response:

1. `model_hash`
2. `resolve_hash`
3. `committed_configuration_id`
4. `working_configuration_id`
5. `dirty_diff_hash`

### 6.7 `set_auto_reset_policy` and `get_auto_reset_policy`

Policy fields:

1. `enabled` (global default remains `true`)
2. `default_timeout_ms`
3. `per_path_overrides[]`
4. `policy_revision`

### 6.8 Sync Operations

`check_for_updates`, `pull_updates`, and `get_sync_status` return:

1. `sync_state`: `idle|checking|pulling|applying|error|offline`
2. `last_successful_sync_utc`
3. `pending_update_summary?`
4. `sync_diagnostics[]`

`pull_updates` also returns:
1. `applied_paths[]`
2. `conflict_paths[]` (changed keys where local dirty existed and upstream override won)

### 6.9 Event Subscription

`subscribe_events` supports:

1. `from_sequence` cursor
2. `max_events`
3. `event_kinds[]`

Event payload always includes:

1. `event_id`
2. `sequence`
3. `event_kind`
4. `scope`
5. `path?`
6. `actor?`
7. `old_value_hash?`
8. `new_value_hash?`
9. `timestamp_utc`

### 6.10 `push_audit_events`

Request:
1. `runtime_snapshot`
2. `backend_connected`
3. `max_events`

Response:
1. `pushed_event_ids[]`
2. `pushed_count`
3. `pending_count`
4. `last_uploaded_sequence`

Semantics:
1. Audit events are persisted locally before push attempts.
2. Offline push attempts are non-destructive and deterministic.
3. Online push advances an idempotent upload cursor (`audit_uploaded_sequence`) only for acknowledged sequence prefix.

### 6.11 `export_pending_sync_bundle`

Request:
1. `runtime_snapshot`
2. `max_audit_events`

Response:
1. `bundle.bundle_id`
2. `bundle.committed_configuration_id`
3. `bundle.working_configuration_id`
4. `bundle.dirty_paths[]`
5. `bundle.pending_audit_events[]`
6. `bundle.pending_audit_count`

Semantics:
1. Export is read-only and deterministic for equivalent snapshot state.
2. Bundle payload is suitable for later manual backend reconciliation.
3. Direct push and backend sync use the same lineage/audit data model.

## 7. Diagnostic Families (Frozen for v2)

Existing families are retained:

1. `E_RUNTIME_OPEN_*`
2. `E_RUNTIME_UNKNOWN_*`
3. `E_RUNTIME_TYPE_MISMATCH`
4. `E_RUNTIME_LIMIT_VIOLATION`
5. `E_RUNTIME_LIFECYCLE_IMMUTABLE`
6. `E_RUNTIME_ARTIFACT_UNKNOWN`
7. `E_RUNTIME_CLI_*`

New v2 families are reserved/frozen:

1. `E_RUNTIME_DIRTY_*`
2. `E_RUNTIME_COMMIT_*`
3. `E_RUNTIME_SYNC_*`
4. `E_RUNTIME_PERSIST_*`
5. `E_RUNTIME_TELEMETRY_*`
6. `E_RUNTIME_SECURITY_*`
7. `E_RUNTIME_EVENT_*`

## 8. Compatibility Strategy (v1 -> v2)

1. v1 commands continue unchanged and map into v2 runtime core.
2. v1 `set_parameter` maps to v2 dirty-write semantics with synthetic single-write transaction context.
3. v1 responses omit v2-only fields unless explicitly requested with `schema_version = 2`.
4. Any v1-to-v2 projection must preserve deterministic diagnostics and hash lineage (`model_hash`, `resolve_hash`).

## 9. Hash and Delta Protocol Binding

Runtime v2 contract requires path-granular identity and delta manifests.
Normative hashing and delta rules are defined in:

`docs/runtime-v2-hash-delta-protocol.md`

## 10. Non-Ambiguity Requirements

The following behavior is explicitly defined to avoid ambiguous transitions:

1. Single-writer mutation ordering for dirty/commit/rollback transitions.
2. Timer expiry vs write race resolution by generation number and monotonic sequence.
3. Reboot recovery applies journal replay before accepting new mutations.
4. Sync apply is upstream-authoritative for changed keys only.
5. Commit always emits explicit base/target configuration identifiers.
