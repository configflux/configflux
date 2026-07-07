# Runtime v2 State Machine Specification

Status: frozen
Date: 2026-02-14
Depends on: `docs/runtime-v2-contract.md`, `docs/runtime-v2-hash-delta-protocol.md`

## 1. State Model

Runtime state is represented by:

1. `baseline_snapshot` (immutable for session)
2. `committed_overlay` (persisted local approved overrides)
3. `dirty_overlay` (uncommitted writes with metadata)
4. `sync_state` (`idle|checking|pulling|applying|error|offline`)
5. `event_sequence` (monotonic global sequence)
6. `dirty_generation` (monotonic per-path ordering token)

Effective value:

1. `effective(path) = dirty_overlay[path] ?? committed_overlay[path] ?? baseline_snapshot[path]`

## 2. Invariants

1. Every mutating transition is serialized through one runtime mutation lock.
2. `event_sequence` strictly increases per emitted event.
3. `dirty_generation` strictly increases for each successful write to a path.
4. `commit_configuration` clears committed dirty entries atomically.
5. Sync apply is upstream-authoritative for changed keys only.

## 3. Transition Table

| Transition | Preconditions | State Effects | Emitted Events |
| --- | --- | --- | --- |
| `runtime_open` | Valid snapshot + hashes | Initialize overlays and metadata | `runtime_opened` |
| `set_parameter` | Path exists, lifecycle mutable, value valid | Update `dirty_overlay[path]`, assign metadata and deadline | `parameter_changed`, `dirty_state_changed` |
| `set_parameters_atomically` | All writes valid | Apply all dirty entries or none | `parameter_changed*`, `dirty_state_changed` |
| `timer_expiry(path, gen)` | Path dirty and `gen == current_generation(path)` | Remove dirty entry for path | `auto_reset`, `dirty_state_changed` |
| `rollback_dirty(all)` | Any dirty entries exist | Clear all dirty entries | `rollback_applied`, `dirty_state_changed` |
| `rollback_dirty(subset)` | Paths subset valid | Clear selected dirty entries | `rollback_applied`, `dirty_state_changed` |
| `commit_configuration` | Runtime not applying another commit/sync | Move effective changed values into committed overlay, clear covered dirty entries, update IDs | `commit_applied`, `dirty_state_changed` |
| `check_for_updates` | Sync enabled | `sync_state = checking` | `sync_state_changed` |
| `pull_updates` | Update metadata available | `sync_state = pulling` | `sync_state_changed` |
| `apply_delta` | Base/hash preconditions pass | Apply changed keys, preserve unchanged keys, update commit IDs | `sync_apply_started`, `sync_apply_completed` |
| `apply_delta_conflict` | Local dirty on changed key | Upstream value wins for changed key | `sync_conflict_resolved` |
| `startup_recover` | Journal exists | Replay committed journal order, reconstruct timers and dirty metadata | `recovery_completed` |
| `startup_recover_corrupt` | Corrupt journal segment detected | Fail closed for segment, keep last known-good prefix | `recovery_warning` |

## 4. Deterministic Ordering Rules

1. Mutating operations are linearized by acquisition of the runtime mutation lock.
2. Timer callbacks validate `dirty_generation` token before applying reset.
3. Sync apply and local commit cannot overlap; one must finish before the other starts.
4. Event emission order matches commit order of state mutation.

## 5. Race Conditions and Resolution

Timer vs write race:

1. Timer carries snapshot of `dirty_generation(path)` at scheduling time.
2. If generation differs at expiry, timer event is ignored.
3. If generation matches, reset proceeds.

Commit vs sync apply race:

1. First mutator holds lock and completes.
2. Second mutator recomputes preconditions against updated configuration IDs.
3. Base mismatch errors are deterministic (`E_RUNTIME_SYNC_BASE_MISMATCH` or `E_RUNTIME_COMMIT_BASE_MISMATCH`).

Rollback vs write race:

1. Serialized by lock.
2. Later operation sees post-state of earlier operation and emits events accordingly.

## 6. Crash Window Semantics

Mutating operations follow write-ahead behavior:

1. Append intent to journal.
2. Fsync journal according to durability profile.
3. Apply mutation in memory.
4. Emit events and update metadata.
5. Persist compacted overlay snapshot asynchronously or on configured threshold.

Crash behavior:

1. Crash before step 2: mutation absent after recovery.
2. Crash after step 2 and before step 3: recovery replays intent.
3. Crash after step 3 and before overlay compaction: recovery replays idempotently.

## 7. Event Invariants

1. Every state mutation emits at least one event with unique `event_id`.
2. Event payload includes `event_sequence`, `scope`, and relevant path set.
3. Event replay from cursor `N` returns all events with sequence `> N` in ascending order.

## 8. Sync Apply Invariants

1. Only `changed_paths` from manifest are mutated.
2. Unchanged keys preserve prior leaf hashes.
3. Local dirty values on changed paths are overridden by upstream value.
4. Apply operation is atomic with respect to readers and writers.
