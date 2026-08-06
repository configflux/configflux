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
3. The batch is checked against the model's declared constraints as a whole,
   not write by write. Writes that are individually valid but jointly violate
   a constraint are rejected together.
4. On a constraint rejection, `applied_count` is `0`, `dirty_generation_max`
   is `0`, no snapshot is returned, and `rejected_paths` lists the writes that
   participate in the violation.

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
3. Direct push and backend sync use the same lineage/audit data model, specified in section 11.

### 6.12 Constraint enforcement on writes

`set_parameter`, `set_parameters_atomically` and `commit_configuration`
enforce the declared constraints of the compiled model. Enforcement is
identical on the CLI and on the C ABI.

A write is evaluated against the session's total known assignment: the facet
selections the session was opened with, updated by every write made during the
session, whether dirty or committed. A later write is therefore checked
against the values earlier writes established, not against the state at open.

Semantics:

1. Type, limit, lifecycle, path and artifact validation run first. A request
   that fails any of them reports that failure, not a constraint failure.
2. A write is rejected only when no assignment of the remaining unset facets
   satisfies the model. Facets that have neither been selected nor written are
   unconstrained by this check, so a write is never rejected on the strength of
   a facet nothing has committed to.
3. A rejection reports `E_SELECTION_CONFLICT` with `status = error` and exit
   code `2`. `entity_path` carries `constraints/<id>` naming the violated
   constraint, and the message quotes its condition. The response also carries
   an `unsat_core` object identifying the rejected selection and the minimal
   set of constraints that conflict with it — the same object
   `explain-rejection` returns. The field is present only on a constraint
   rejection and omitted otherwise.
4. A rejected operation changes nothing: no snapshot is returned and the
   caller's snapshot remains valid.
5. `rollback_dirty` is not constraint-checked. With
   `set_parameters_atomically`, it is the way to leave a state that no single
   write can leave.
6. Enforcement covers the runtime CLI and the runtime C ABI, so a C++ or ROS2
   caller is checked identically. Writes made through the edge agent are not
   constraint-checked; the agent operates on a pre-resolved snapshot without a
   solver session.
7. Parameters map to facets by the final segment of their path, so two
   different parameters can map to the same facet. When they do and their
   values in the session disagree, that facet is excluded from the known
   assignment the write is evaluated against — it is treated as unset, so the
   write is not rejected on its account. When those values agree, the agreed
   value is used and the write is evaluated normally. A model whose components
   share parameter names should therefore not rely on write-time enforcement
   for those facets. The runtime test suite's `run_060` pins this behavior.

## 7. Diagnostic Families (Frozen for v2)

Existing families are retained:

1. `E_RUNTIME_OPEN_*`
2. `E_RUNTIME_UNKNOWN_*`
3. `E_RUNTIME_TYPE_MISMATCH`
4. `E_RUNTIME_LIMIT_VIOLATION`
5. `E_RUNTIME_LIFECYCLE_IMMUTABLE`
6. `E_RUNTIME_ARTIFACT_UNKNOWN`
7. `E_RUNTIME_CLI_*`
8. `E_SELECTION_*` — emitted by the write and explain paths when the solver
   adjudicates a selection against the model's declared constraints. Shared
   with the interpreter and `cfx` so one taxonomy describes a constraint
   violation wherever it is reported.

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
6. Constraint enforcement on a write evaluates the session's total known
   assignment — open-time selections updated by all in-session writes — and
   rejects only when no completion of the unset facets satisfies the model.

## 11. Provenance Lineage Data Model

The provenance lineage is a content-addressed, parent-linked chain that pins the
versioned state of a unit over time. It is the serializable data model referred
to by the sync and audit operations of sections 6.8-6.11 ("the same lineage/audit
data model"). Three serializable types make up the format; the field names below
are the on-the-wire JSON keys.

This section specifies the format only — payload shape, field semantics,
canonicalization, and the content-addressing rule. It defines no runtime
behavior, control flow, or lifecycle beyond serialization and hashing.

### 11.1 `ProvenanceVersionTriple`

The versioned state a lineage entry pins. Each axis is a version-identity string
— typically a SHA-256 hex, consistent with how `model_hash`, `resolve_hash`, and
the configuration-identity fields are represented elsewhere in this contract.

1. `model_version` (string): the model version the unit was resolved from.
2. `selection_version` (string): the preselection (facet/option) version applied.
3. `override_layer` (string): the working-overlay identity layered on top of the
   resolved output.

All three fields are always present. The triple carries state only; it holds no
actor, reason, timestamp, or intent.

### 11.2 `ProvenanceLineageEntry`

One entry in a unit's lineage: a pinned `ProvenanceVersionTriple` together with
who/why/when it was recorded, a parent-pointer, and the entry's own content
address. Fields, in wire order:

1. `entry_id` (string, 64-char lowercase SHA-256 hex): the content address of
   this entry (section 11.4). Always present on the wire.
2. `state` (`ProvenanceVersionTriple`): the versioned state this entry pins.
3. `actor` (string): the identity that produced this state.
4. `reason` (string, optional): why the state came to be. Omitted from the wire
   form when absent.
5. `timestamp_unix_ms` (integer, unsigned 64-bit): when the state was recorded,
   in Unix milliseconds.
6. `intent` (string enum): the entry's override-layer intent — one of
   `experimental` (the default) or `compensating`. Always present on the wire.
7. `parent_entry_id` (string, optional): the `entry_id` of the prior entry in the
   chain. Omitted from the wire form at the chain root.

Wire-serialization rules:

1. `reason` and `parent_entry_id` are omitted from the serialized entry when
   absent (no `null` key on the wire).
2. `intent` is always serialized. An entry persisted before the field existed
   deserializes as `experimental`.
3. `entry_id` is always serialized and equals the content address recomputed from
   the entry's other fields (section 11.4).

### 11.3 `ProvenanceLineage`

The serializable carrier for a unit's lineage.

1. `entries` (array of `ProvenanceLineageEntry`): an ordered, parent-linked chain.
   Each entry after the root references its predecessor through
   `parent_entry_id`. Defaults to an empty array when absent.

The carrier imposes no semantics beyond holding the entries in order.

### 11.4 Content addressing (`entry_id`)

`entry_id` is a deterministic content address: the lowercase SHA-256 hex digest
of a *canonical payload* derived from the entry's contents. The canonical payload
differs from the wire form (section 11.2) in four ways:

1. It is prefixed with a `schema_version` field set to the current product schema
   version (value `4`). This field does not appear in the serialized entry; it is
   injected into the canonical payload before hashing. It is distinct from the
   Runtime envelope `schema_version` of section 2 (value `2`). The pin ties an
   entry's address to the schema version in force when it was computed; entries
   already addressed under an earlier version keep their addresses.
2. It **excludes `entry_id`** — a content address cannot depend on itself.
3. It fixes the field order to: `schema_version`, `state`, `actor`, `reason`,
   `timestamp_unix_ms`, `intent`, `parent_entry_id`. Within `state`, the order is
   `model_version`, `selection_version`, `override_layer`.
4. The optional fields `reason` and `parent_entry_id` are **always present** in
   the canonical payload, serialized as JSON `null` when absent — unlike the wire
   form, which omits them. Presence and absence are therefore unambiguous in the
   hashed bytes.

The canonical payload is serialized as compact JSON (UTF-8, no insignificant
whitespace, keys in the fixed order above), and `entry_id` is the SHA-256 of
exactly those bytes rendered as 64 lowercase hex characters — the same digest
form as `model_hash`, `resolve_hash`, and `committed_configuration_id`.

Consequences of the rule:

1. `parent_entry_id` is part of the hashed payload, so re-parenting an otherwise
   identical entry yields a different `entry_id`.
2. Any change to `state`, `actor`, `reason`, `timestamp_unix_ms`, or `intent`
   changes the `entry_id`.
3. A verifier can recompute `entry_id` from an entry's contents alone and compare;
   no external index is required.

#### Worked example

A root entry — `state = { model_version: "m1", selection_version: "s1",
override_layer: "o1" }`, `actor = "operator-a"`, `reason = "initial state"`,
`timestamp_unix_ms = 1700000000000`, `intent = experimental`, and no parent — has
this canonical payload (the normative byte layout):

```json
{"schema_version":4,"state":{"model_version":"m1","selection_version":"s1","override_layer":"o1"},"actor":"operator-a","reason":"initial state","timestamp_unix_ms":1700000000000,"intent":"experimental","parent_entry_id":null}
```

The SHA-256 of exactly those bytes is its `entry_id`:

```
31619b2f83be1c177014dcae9a176cf6eca26bb85f627ff3722593b197115583
```

A child entry that pins `model_version = "m2"` (other state axes unchanged),
`actor = "operator-a"`, no `reason`, `timestamp_unix_ms = 1700000005000`,
`intent = compensating`, and `parent_entry_id` set to the root's `entry_id` above
has canonical payload:

```json
{"schema_version":4,"state":{"model_version":"m2","selection_version":"s1","override_layer":"o1"},"actor":"operator-a","reason":null,"timestamp_unix_ms":1700000005000,"intent":"compensating","parent_entry_id":"31619b2f83be1c177014dcae9a176cf6eca26bb85f627ff3722593b197115583"}
```

and `entry_id`:

```
456f2cdd1b13921a30a69a9aa68ff59bfc7367241a9e1e8d202268e174ba85ea
```

The child illustrates both optional-field rules: `reason` serializes as `null`
(absent) and `parent_entry_id` carries the root's address, both inside the hashed
bytes.
