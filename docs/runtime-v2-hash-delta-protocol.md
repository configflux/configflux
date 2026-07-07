# Runtime v2 Hash-Tree and Delta Commit Protocol (Frozen)

Status: frozen
Date: 2026-02-14
Normative parent: `docs/runtime-v2-contract.md`

## 1. Purpose

This protocol defines deterministic path-granular hashing and delta manifests for:

1. Runtime commit artifacts
2. Backend sync pull/apply
3. Direct-push commissioning

API-level commit remains full-state semantic. Storage and transport remain delta-first.

## 2. Canonical Path and Payload Rules

Path canonicalization:

1. Path format is `<scope>/<parameter_path>` with `/` separators.
2. No trailing slash, no duplicate separators.
3. Path ordering is bytewise lexical order.

Leaf canonical payload fields (required):

1. `path`
2. `value_type`
3. `value_canonical`
4. `unit?`
5. `lifecycle`
6. `bounds?`
7. `artifact_ref?`

Canonical serialization:

1. UTF-8 JSON
2. Sorted object keys
3. Deterministic number formatting (no exponent normalization drift)
4. No insignificant whitespace

## 3. Leaf and Root Hash Derivation

Leaf hash:

1. `leaf_hash(path) = sha256(canonical_leaf_json_bytes)`

Root derivation:

1. Build ordered vector of `path + ":" + leaf_hash` for all effective committed paths.
2. Concatenate entries with newline (`\n`) separators.
3. `root_hash = sha256(concatenated_bytes)`.

Identity mapping:

1. `committed_configuration_id = root_hash(committed effective set)`
2. `working_configuration_id = root_hash(working effective set)`
3. `dirty_diff_hash = sha256(canonical dirty overlay object)`

## 4. Delta Manifest Schema

Each commit or sync apply artifact emits:

1. `schema_version`
2. `manifest_id`
3. `base_configuration_id`
4. `target_configuration_id`
5. `changed_paths[]`
6. `created_at_utc`
7. `actor`
8. `reason?`

`changed_paths[]` entry:

1. `path`
2. `change_kind` (`set|delete|metadata`)
3. `before_leaf_hash` (nullable for adds)
4. `after_leaf_hash` (nullable for deletes)
5. `before_value?`
6. `after_value?`

Rules:

1. `changed_paths[]` is sorted by canonical path.
2. Unchanged paths are not included.
3. `before_leaf_hash` must match the receiver's base view for successful apply.

## 5. Apply Semantics

Delta apply preconditions:

1. Receiver has `base_configuration_id` equal to manifest base.
2. All `before_leaf_hash` checks pass.
3. Runtime policy validation succeeds for every changed path.

On success:

1. Apply all changes atomically.
2. Derive resulting target root and verify `target_configuration_id`.
3. Emit sync/commit event with manifest metadata.

On failure:

1. No partial apply.
2. Return deterministic diagnostics in `E_RUNTIME_SYNC_*` or `E_RUNTIME_COMMIT_*`.

## 6. Rebase and Full-Snapshot Fallback

When base mismatch occurs:

1. Receiver returns `E_RUNTIME_SYNC_BASE_MISMATCH`.
2. Sender may provide rebase manifest from receiver base.
3. If rebase cannot be produced safely, sender issues full snapshot.

Full snapshot is allowed only for:

1. Initial bootstrap
2. Unrecoverable divergence
3. Explicit operator override

## 7. Direct Push and Backend Parity

1. Direct push uses the same manifest schema by default.
2. Backend sync uses identical validation semantics.
3. Both paths share hash derivation and deterministic diagnostics.

## 8. Deterministic Guarantees

1. Same canonical input state yields the same leaf and root hashes across process restarts.
2. Hash derivation is independent of map insertion order.
3. Delta manifests are replay-safe and idempotent with manifest ID + base/target tuple.

## 9. Required Diagnostics

Required protocol diagnostics:

1. `E_RUNTIME_SYNC_BASE_MISMATCH`
2. `E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH`
3. `E_RUNTIME_SYNC_TARGET_HASH_MISMATCH`
4. `E_RUNTIME_COMMIT_BASE_MISMATCH`
5. `E_RUNTIME_COMMIT_TARGET_HASH_MISMATCH`
6. `E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED`
