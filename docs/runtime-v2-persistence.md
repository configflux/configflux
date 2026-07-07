# Runtime v2 Persistence Format and Crash-Recovery Journal

Status: frozen
Date: 2026-02-14
Depends on: `docs/runtime-v2-state-machine.md`

## 1. Scope

This specification defines on-disk persistence schemas, write-ahead ordering, crash recovery, and bounded storage behavior for Runtime v2.

## 2. Persistence Files

Runtime persistence root contains:

1. `baseline.snapshot.json`
2. `committed.overlay.json`
3. `dirty.overlay.journal`
4. `runtime.meta.json`

Each file has:

1. `schema_version`
2. `format_version`
3. `created_at_utc` and `updated_at_utc`
4. `checksum` over canonical payload

## 3. File Schemas

### 3.1 `baseline.snapshot.json`

Required fields:

1. `model_hash`
2. `resolve_hash`
3. `scope`
4. `parameters[]` canonical resolved values

Rules:

1. Immutable after successful `runtime_open` for a session.
2. Replaced only by explicit re-open/bootstrap flow.

### 3.2 `committed.overlay.json`

Required fields:

1. `committed_configuration_id`
2. `entries[]`: `{ path, value, value_type, leaf_hash }`
3. `last_commit_id`
4. `last_commit_at_utc`

### 3.3 `dirty.overlay.journal`

Append-only records:

1. `record_id`
2. `sequence`
3. `op` (`set_dirty|clear_dirty|rollback|commit|sync_apply`)
4. `payload`
5. `checksum`

Rules:

1. Records are canonical JSON lines (one record per line).
2. Record checksum is validated during replay.
3. Corrupt records terminate replay at last valid prefix.

### 3.4 `runtime.meta.json`

Required fields:

1. `event_sequence`
2. `dirty_generation_counters` (per path)
3. `auto_reset_policy_revision`
4. `sync_cursor`
5. `durability_profile`

## 4. Write-Ahead Ordering

For each mutating operation:

1. Build intent record(s) in memory.
2. Append intent to `dirty.overlay.journal`.
3. Fsync journal according to durability profile.
4. Apply mutation in memory.
5. Update `runtime.meta.json` with sequence/generation changes.
6. Periodically checkpoint `committed.overlay.json` and compact journal.

Durability profiles:

1. `strict`: fsync journal on every mutation
2. `balanced`: fsync every batch or timeout
3. `relaxed`: fsync on checkpoint or graceful shutdown

## 5. Startup Recovery

Recovery order:

1. Load and validate `baseline.snapshot.json`.
2. Load and validate `committed.overlay.json`.
3. Load `runtime.meta.json` if present.
4. Replay `dirty.overlay.journal` in record order.
5. Reconstruct dirty timers and generation counters.
6. Recompute configuration IDs and verify consistency.

## 6. Corruption Handling

Failure modes and behavior:

1. Invalid baseline checksum: fail closed, runtime open rejected.
2. Invalid committed overlay checksum: fail closed, recovery halted.
3. Dirty journal partial corruption: replay valid prefix, emit deterministic warning.
4. Invalid metadata checksum: regenerate metadata from overlays and replay state.

Diagnostics:

1. `E_RUNTIME_PERSIST_CHECKSUM_MISMATCH`
2. `E_RUNTIME_PERSIST_RECORD_CORRUPT`
3. `E_RUNTIME_PERSIST_RECOVERY_ABORTED`
4. `E_RUNTIME_PERSIST_RECOVERY_PARTIAL`

## 7. Migration and Versioning

1. `format_version` controls on-disk migration path.
2. Runtime supports explicit one-step migration from `format_version = 1` to `2`.
3. Unsupported versions fail with deterministic diagnostics and no partial writes.

## 8. Bounded Storage Policy

1. Journal compaction runs when record count or byte threshold is exceeded.
2. Compaction writes new overlay snapshot, fsyncs, then atomically swaps files.
3. Retain at most `N` archived journals with deterministic pruning (oldest first).
4. Optional compression is allowed for archived journals only.

## 9. fsync Boundaries and Atomicity

Atomicity guarantees:

1. Commit is visible only after journal append and metadata update complete.
2. Checkpoint publish uses atomic rename for snapshot replacement.
3. Readers always see last completed checkpoint plus replayed in-memory delta.
