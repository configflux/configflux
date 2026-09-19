# Runtime API and CLI Contract v2 (Frozen)

Status: frozen
Date: 2026-02-14
Supersedes: `docs/runtime-cli-contract.md` for v2 operations

Revisions:

- 2026-09-12 — The `explain-rejection` operation is documented. The runtime has
  shipped the command since M4, and this document described it nowhere: section 5
  listed it under neither operation family and section 6 had no subsection for
  it, so a reader had no way to learn that the operation exists, let alone what
  it accepts. The omission cost more than one missing operation. Its result is
  the single response envelope that does not carry `resolve_hash`, which is why
  section 4 could not put that field in its common list and had to name an
  envelope this document then declined to cover — a callout that pointed at
  nothing a reader could look up. Section 5 now carries the operation in a third
  group, deliberately apart from both numbered family lists, because it reached
  the runtime after the v2 set was frozen (ADR-0031 D1) and a fifteenth entry in
  either list would have misstated which set froze it. Section 6.13 gives it a
  contract: request, response, the nested unsat core, the rule that a rejection
  is a successful response rather than an error (ADR-0031 D2), the condition
  under which the core is present (ADR-0031 D3), and the reason `resolve_hash` is
  absent by design rather than by oversight. Section 4's callout now points at
  that subsection instead of describing an envelope it would not document. Both
  doc-pinning guards carry the exception rather than dropping the assertion that
  found it: the set of envelopes without `resolve_hash` is derived from the
  shipped structs rather than named in a test, so a second envelope losing the
  field fails every pin that reads it, and the examples excused from the field
  must be exactly the envelopes that ship without it.

- 2026-09-11 — Section 6.8's digest rule now says lowercase, because the shipped
  gate now requires it. The runtime's only format check on hash-shaped input
  accepted `A-F`, while every value it compares against is rendered by
  `format!("{:x}", ...)`, so an uppercase digest cleared the check and could then
  never compare equal to anything the runtime computes — and on these four
  fields it reached the equality check and came back as a configuration
  divergence, which names the wrong cause. The gate accepts `0-9a-f` only, the
  field messages it emits name lowercase, and the sentence that used to tell a
  reader an uppercase id "passes the format check and then fails the comparison"
  now records that it is refused on shape instead. Nothing the runtime produces
  changes: lowercase is what every producer already emitted, and the case of a
  supplied value is never normalized on ingest, because rewriting a caller's
  digest would make the recorded hash one the caller did not send.

- 2026-09-10 — Sections 2 and 8 brought to the shipped surface. Section 2 stated
  that `schema_version = 2` identifies a v2 envelope and that
  `schema_version = 1` remains valid for v1 commands, and section 8 said a v1
  response omits v2-only fields "unless explicitly requested with
  `schema_version = 2`". None of that ships, and none of it ever did. Both
  command families are versioned by the single product schema version; every
  handler in both compares the request against it and rejects any other value
  with `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION`, and no code path anywhere accepts
  the literal 2 or branches on the requested version to decide what to emit.
  There is no v1/v2 envelope-version axis in the runtime to describe, so the
  contract no longer claims one, and section 11.4's cross-reference to it went
  with the rule. Two further claims the old text got wrong are now stated
  correctly: a response carries the version the runtime implements rather than
  echoing the caller's — including the rejection envelope sent to a request that
  named some other value — and the top-level envelope is identical across the
  two families, so no field distinguishes them. `unsat_core` and
  `runtime_snapshot` sit on v1 results as well as v2 ones, which is why the
  "v2-only field" framing had to go rather than be repaired.
  `//compiler:runtime_v2_contract_doc_envelope_test` now pins all of it: every
  version this document prints beside `schema_version` must equal
  `PRODUCT_SCHEMA_VERSION`, sections 2 and 8 must state no version of their own
  and must name the diagnostic instead, no section may reintroduce a per-family
  envelope version, and the v1 and v2 response envelopes must still share one
  field set — the last derived from the shipped structs, since the v1/v2 split
  exists only in this document and nowhere in the code.

- 2026-09-07 — Section 6.8 corrected to the preconditions the shipped
  `pull_updates` actually enforces. The section documented one — a stale
  `before_leaf_hash` — and was silent on the two that reject a request before
  it. Every configuration id and leaf hash a `pull_updates` payload carries
  must be exactly 64 ASCII hex characters, and the section's own example values
  were not: submitted verbatim they came back
  `E_RUNTIME_SYNC_PAYLOAD_INVALID`, so the document taught a request the binary
  refuses. Those literals are now real-shaped digests, and
  `//compiler:runtime_v2_contract_doc_section6_test` hands the documented
  request to the shipped `pull_updates` and asserts which gate it reaches, so an
  example the runtime would reject cannot land again. The prose now also states
  that the ids and hashes are verified rather than recorded — the base id
  against the runtime's current committed configuration, each `after_leaf_hash`
  against the value in its own write, and the target id against what the
  runtime computes after applying the batch — naming
  `E_RUNTIME_SYNC_BASE_MISMATCH` and `E_RUNTIME_SYNC_TARGET_HASH_MISMATCH`,
  neither of which section 6 mentioned anywhere. Sections 6.5, 6.6 and 6.11
  carry the same short placeholder ids in fields the runtime does not
  format-check; those are output it cannot produce rather than input it
  refuses, and are tracked separately.

- 2026-09-07 — Sections 6.8-6.11 brought under the same pin as 6.1-6.7. The
  sync, event-subscription, audit-push and bundle-export operations each carry a
  fenced JSON example of every payload they exchange, and
  `//compiler:runtime_v2_contract_doc_section6_test` round-trips all of them
  through the shipped structs, so section 6 no longer has an unguarded half. The
  prose lists those subsections used to carry omitted fields the runtime does
  emit: the four sync fields nest under an optional `sync_status` rather than
  sitting at the top level, `pull_updates` carries an actor, a source, a write
  list and the configuration identifiers on both sides, `subscribe_events`
  returns a cursor pair and a dropped-event count alongside its events, and the
  offline bundle carries a schema version, a generation timestamp, an upload
  cursor and a sync status. Section 6.12 stays example-free because it
  introduces no payload type of its own. Prose claims the runtime does not honor
  were corrected in the same pass: `next_sequence` reports the stream head and is
  not the cursor to send on the next `subscribe_events` call, and a conflicting
  `pull_updates` raises one aggregate warning rather than one per conflicting
  path. Every identifier the runtime derives by formatting a number was also
  re-rendered as the runtime writes it: a runtime event's `event_id` and an audit
  event's are their sequence in sixteen hex digits behind `evt-` and `audit-`
  respectively, not the decimal the examples had, and a bundle's `bundle_id` is
  `offline-sync-` plus sixteen hex characters of a content hash rather than a
  counter.

- 2026-09-06 — Section 6.1-6.7 corrected to the shipped request and response
  shapes. The frozen text named fields the runtime does not accept: `scope`
  where the request field is `scope_root`, a scope selector on
  `get_dirty_metadata` that does not exist, per-write `actor`/`reason` that
  belong to the batch, and flat dirty-metadata, configuration-identity and
  auto-reset-policy fields that the responses nest. Each operation now carries a
  fenced JSON example of the payload it actually exchanges, and
  `//compiler:runtime_v2_contract_doc_section6_test` round-trips every one of
  them through the shipped structs so the two cannot diverge again. That
  revision left sections 6.8-6.12 prose-only and unguarded, with no fenced
  examples of their own; their timestamp fields were corrected in place (`last_successful_sync_unix_ms` and
  `timestamp_unix_ms`, both Unix milliseconds), and `subscribe_events` reports an
  affected path inside the event payload rather than at the top level. That pass
  also left sections 2 and 8 describing `schema_version = 2` for v2 envelopes, a
  rule the shipped runtime never had; the 2026-09-10 revision above corrects
  them.

## 1. Scope

This document freezes Runtime v2 request/response contracts for:

1. Dirty metadata operations
2. Full-state commit and rollback
3. Auto-reset policy control
4. Sync status and update pull/apply
5. Event subscription and deterministic event replay

Runtime v1 commands remain supported for backward compatibility.

## 2. Versioning Rules

1. Both command families are versioned by one value: the current product schema
   version, defined once as `PRODUCT_SCHEMA_VERSION` in
   `compiler/src/product_api.rs`. A v1 request and a v2 request carry the same
   `schema_version`, and the runtime has no second version axis — no value
   distinguishes the two families, on the wire or anywhere else.
2. Every request is checked against that value before its payload is read, and
   any other value is rejected with `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION`. The
   check is an equality, and it is the same check in both families: a lower
   version is refused exactly as firmly as a higher one, because there is no
   compatibility mode and no dual-accept.
3. A persisted `runtime_snapshot` carries the same version and is checked the
   same way, raising the same code when a snapshot outlives the build that
   wrote it.
4. Every response carries the current product schema version, including the
   rejection envelope returned to a request that named some other value. A
   response reports the version the runtime implements, not the one the caller
   asked for.
5. Runtime-core semantics are defined by this document; CLI transport behavior remains deterministic and follows v1 transport invariants.

## 3. Deterministic Transport and Exit Codes

Transport behavior remains byte-deterministic for equivalent request bytes.

Exit code mapping is unchanged:

1. `0`: response `status = ok`
2. `2`: response `status = error` (domain validation/runtime rejection)
3. `1`: CLI/transport failure (`args`, I/O, malformed request, oversized request)

## 4. Common Envelope Fields

Every Runtime v2 response carries these top-level fields. The list is the
intersection of the top-level fields of every response envelope in
`compiler/src/runtime_api/contracts.rs`, and
`//compiler:runtime_v2_contract_doc_envelope_test` derives that intersection
from those structs and asserts this list equals it, so the two cannot drift
apart in either direction.

1. `schema_version`
2. `status` (`ok` or `error`)
3. `model_hash`
4. `scope`
5. `error_count`
6. `warning_count`
7. `diagnostics`
8. `diagnostics_ref` (optional)

Only `diagnostics_ref` is optional; the other seven are always present. Like
every optional field in this document it is omitted entirely when absent, never
sent as `null`. `diagnostics` is the report object rather than a bare array: it
nests its own `schema_version`, `diagnostics[]`, `error_count` and
`warning_count`, and the envelope's own two counts repeat the report's.

`resolve_hash` is carried by the response of every operation section 5 lists,
but it is not universal across the shipped envelopes:
`RuntimeExplainRejectionResult`, the envelope of the `explain-rejection`
operation section 6.13 documents, omits it.

Three names that are *not* envelope fields. `committed_configuration_id` and
`working_configuration_id` belong to `RuntimeConfigurationIdentity` (reached
through `get_configuration_identity`), to `OfflineReconciliationBundle` (through
`export_pending_sync_bundle`) and to `RuntimeAuditEvent` (nested in that
bundle's `pending_audit_events`). They are never top-level fields of a response.
No field named `event_sequence` exists anywhere in the contracts: an event's
sequence number is `sequence` on a `RuntimeEvent`, and `from_sequence` /
`next_sequence` bound the range a `subscribe_events` response returns
(section 6.9).

Determinism rules:

1. The `diagnostics[]` array inside the report has a stable, deterministic
   order.
2. Path lists are sorted in canonical lexical order.
3. Event sequence numbers are monotonic within a session. They live in
   `runtime_snapshot`, so they continue across process boundaries exactly as far
   as the caller round-trips that snapshot; the runtime itself persists nothing.

## 5. Runtime v2 Operation Set

Runtime v1 operations remain:

1. `runtime_open`
2. `get_scope_metadata`
3. `list_parameters`
4. `get_parameter`
5. `set_parameter`

Path grammar (ADR-0057 §D7). `list_parameters` enumerates writable parameter
paths, `component.<component_id>.param.<param_key>`, and every write verb
accepts exactly those. `get_parameter` additionally reads one field of a
delivered catalogue entry at
`component.<component_id>.requires.<slot>.<field>`, returning the usual payload
plus a `requires` block naming the slot, binding and entry. Those paths are
read-only in this version: the value was fixed at resolve time, so a write to
one is refused with `E_RUNTIME_UNKNOWN_PATH`, and they are absent from
`list_parameters` because nothing may set them.

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

One further operation belongs to neither list. `explain-rejection` reached the
runtime with M4, months after the fourteen operations above were frozen as the
v2 set, and it was never part of the set this document froze. ADR-0031 D1 adds
it as a read-side, solver-decided query: it asks why setting a parameter to a
candidate value would be refused, answers from the same compiled model the write
verbs are checked against, and mutates nothing. Its CLI spelling is kebab-case,
`explain-rejection`, following the runtime's convention for a multi-word command
name (`set-parameter`, `get-configuration-identity`); it is listed below in the
snake_case form the rest of this section uses. Its response envelope is the
single exception to the `resolve_hash` note in section 4 —
`RuntimeExplainRejectionResult` is the only shipped envelope that omits that
field, by ADR-0031 D2's design. Section 6.13 carries its contract.

Added after the v2 set was frozen:

1. `explain_rejection`

## 6. Operation Contracts (v2 Additions)

Shapes below are the shipped ones. Sections 6.1-6.11 and 6.13 carry a fenced JSON
example of every payload they describe, and `//compiler:runtime_v2_contract_doc_section6_test`
extracts each one and round-trips it through the struct the runtime actually
deserializes, so no subsection here can drift from the binary in either
direction. Section 6.12 has no example of its own because it introduces no
payload type: it states enforcement semantics over payloads that 6.1 and 6.5
already pin.

Conventions used throughout this section:

1. Every v2 request carries `schema_version` set to the current product schema
   version. Any other value is rejected with
   `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION`.
2. Every v2 request also carries `runtime_snapshot`, the snapshot returned by
   the previous response — the runtime CLI is stateless per process. The request
   examples below **elide** that field; it is required on the wire.
3. Every response carries the common envelope fields of section 4. Each
   response example below shows all of them except the optional
   `diagnostics_ref`, which no example exercises. Each also carries
   `resolve_hash` (section 4) with exactly one exception: the 6.13 response
   example, whose `RuntimeExplainRejectionResult` is the only shipped envelope
   that omits that field.
4. Optional fields are omitted from a response entirely when absent, never sent
   as `null`.
5. Every hash and configuration-id value in the examples is a full
   64-character lowercase sha256 hex digest. That is the only form the runtime
   emits, and — for the `pull_updates` ids and leaf hashes of section 6.8 —
   the only form it accepts. The identifiers that are not hashes (`commit_id`,
   `bundle_id`, `event_id`, `audit_event_id`) are shown in the derived form
   their own subsection documents.

Path forms. A request names a parameter by its bare path,
`component.<component_id>.param.<param_key>`; the runtime finds the owning scope
root itself. A request may also give the scope-qualified form,
`<scope_root>/component.<component_id>.param.<param_key>`, and both are accepted
wherever a request takes a path (`rollback_dirty.paths`,
`commit_configuration.changed_paths_hint`, `get_dirty_metadata.path`).

Responses are not symmetric with requests here. Every path list a response
*returns* is scope-qualified — `list_dirty_parameters.dirty_paths`,
`rollback_dirty.rolled_back_paths`, `rollback_dirty.remaining_dirty_paths` and
`commit_configuration.changed_paths[].path` are all rendered as
`<scope_root>/<path>`. The one exception is
`set_parameters_atomically.rejected_paths`, which echoes the caller's supplied
path verbatim so a rejection can be matched against the request that caused it.
A caller comparing a rejected path against a dirty path must therefore normalize
one of the two.

### 6.1 `set_parameters_atomically`

`actor` and `reason` are properties of the batch and live on the request. A
`writes[]` entry carries only the path and the value.

Request (`SetParametersAtomicallyRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "writes": [
    {
      "path": "component.heat_exchanger.param.setpoint_trim",
      "value": 0.42
    }
  ],
  "actor": "operator-a",
  "reason": "commissioning trim",
  "expected_working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
  "intent": "experimental"
}
```

`expected_working_configuration_id` makes the batch a compare-and-set against
the session's current working identity whenever it is **present**. Omit the
field, or send `null`, to make no claim about the current identity. A blank or
whitespace-only value is not an omission: it is a present expectation the
runtime cannot honour, and is refused as malformed input with
`E_RUNTIME_DIRTY_INVALID` against that field — as is any other value that is not
a 64-character lowercase sha256 hex string, rather than being reported as a
mismatch against an identity it could never equal. The value is compared exactly
as sent; surrounding whitespace is not stripped.

This operation accepts `expected_working_configuration_id` and no other
expected-id field. A request carrying `expected_base_configuration_id`, the name
`commit_configuration` uses (section 6.5), is refused as an unrecognized field
with `E_RUNTIME_CLI_REQUEST_INVALID` rather than ignored, so an expectation
aimed at the wrong operation cannot pass for no expectation at all.
`intent` is the governance intent recorded on every path the batch dirties; it
defaults to `experimental` when omitted.

Nested (`AtomicParameterWrite`):

```json
{
  "path": "component.heat_exchanger.param.setpoint_trim",
  "value": 0.42
}
```

Response (`SetParametersAtomicallyResult`), a rejection:

```json
{
  "schema_version": 5,
  "status": "error",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "applied_count": 0,
  "rejected_paths": [
    "component.pump_drive.param.control_mode"
  ],
  "dirty_generation_max": 0,
  "error_count": 1,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [
      {
        "code": "E_RUNTIME_LIFECYCLE_IMMUTABLE",
        "severity": "error",
        "message": "Parameter 'component.pump_drive.param.control_mode' is not writable at runtime",
        "entity_path": "request.writes"
      }
    ],
    "error_count": 1,
    "warning_count": 0
  }
}
```

On success `status` is `ok`, `applied_count` is the number of writes,
`rejected_paths` is empty, `dirty_generation_max` is the highest generation the
batch produced, and `runtime_snapshot` carries the updated session state. A
constraint rejection additionally carries `unsat_core` (section 6.12).

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

The scope selector on this request is `scope_root`, and it is required.

Request (`ListDirtyParametersRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "scope_root": "thermal_control"
}
```

Response (`ListDirtyParametersResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "scope_root": "thermal_control",
  "dirty_paths": [
    "thermal_control/component.heat_exchanger.param.setpoint_trim"
  ],
  "dirty_count": 1,
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

`dirty_paths` is sorted, deduplicated, and restricted to the requested scope
root. The response echoes `scope_root` alongside the envelope's `scope`.

### 6.3 `get_dirty_metadata`

This request takes no scope selector: the path identifies the parameter, and the
runtime resolves its scope root and returns it.

Request (`GetDirtyMetadataRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "path": "component.heat_exchanger.param.setpoint_trim"
}
```

Response (`GetDirtyMetadataResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "path": "component.heat_exchanger.param.setpoint_trim",
  "dirty": true,
  "scope_root": "thermal_control",
  "metadata": {
    "actor": "operator-a",
    "reason": "commissioning trim",
    "dirty_since_unix_ms": 1700000000000,
    "reset_deadline_unix_ms": 1700000900000,
    "hard_cap_deadline_unix_ms": 1731536000000,
    "generation": 1,
    "intent": "experimental"
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

`dirty` is the top-level answer. The override's attributes are nested under
`metadata` and are absent when the path is not dirty.

Nested (`DirtyEntryMetadata`):

```json
{
  "actor": "operator-a",
  "reason": "commissioning trim",
  "dirty_since_unix_ms": 1700000000000,
  "reset_deadline_unix_ms": 1700000900000,
  "hard_cap_deadline_unix_ms": 1731536000000,
  "generation": 1,
  "intent": "experimental"
}
```

Timestamps are **Unix milliseconds**, not formatted UTC strings. The two
deadlines are distinct bounds (ADR-0037 Decision 3): `reset_deadline_unix_ms` is
the short session-lease bound, absent when auto-reset is disabled for the path;
`hard_cap_deadline_unix_ms` is the long bound, absent when no hard cap is
scheduled. `reason` is omitted when the write carried none.

### 6.4 `rollback_dirty`

Request (`RollbackDirtyRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "mode": "subset",
  "paths": [
    "component.heat_exchanger.param.setpoint_trim"
  ],
  "actor": "operator-a",
  "reason": "abandon experiment"
}
```

`mode` is `all` or `subset`. `paths` is required for `subset` and ignored for
`all`.

Response (`RollbackDirtyResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "rolled_back_paths": [
    "thermal_control/component.heat_exchanger.param.setpoint_trim"
  ],
  "remaining_dirty_paths": [],
  "rollback_event_id": "rb-0000000000000042",
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

`rollback_event_id` is omitted when the request rolled nothing back. Both path
lists are scope-qualified even though the request named a bare path.

### 6.5 `commit_configuration`

Request (`CommitConfigurationRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "actor": "operator-a",
  "reason": "persist trim",
  "expected_base_configuration_id": "70a8b15a84768a8f922387e61866ab183a2c368516c6b457e73295ec6245354a",
  "changed_paths_hint": [
    "component.heat_exchanger.param.setpoint_trim"
  ]
}
```

`changed_paths_hint` is an assertion, not a selector: commit is full-state, and
a hint naming a path that is not currently dirty fails the commit with
`E_RUNTIME_COMMIT_INVALID`.

`expected_base_configuration_id` makes the commit a compare-and-set against the
session's committed identity whenever it is **present**. Omit the field, or send
`null`, to make no claim about the committed identity. A blank or whitespace-only
value is not an omission: it is a present expectation the runtime cannot honour,
and fails the commit with that same `E_RUNTIME_COMMIT_INVALID` — as does any
other value that is not a 64-character lowercase sha256 hex string, rather than
being reported as a base mismatch. The value is compared exactly as sent;
surrounding whitespace is not stripped.

This operation accepts `expected_base_configuration_id` and no other expected-id
field. A request carrying `expected_working_configuration_id`, the name
`set_parameters_atomically` uses (section 6.1), is refused as an unrecognized
field with `E_RUNTIME_CLI_REQUEST_INVALID` rather than ignored, so an expectation
aimed at the wrong operation cannot pass for no expectation at all.

Response (`CommitConfigurationResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "commit_id": "commit-0000000000000007",
  "base_configuration_id": "70a8b15a84768a8f922387e61866ab183a2c368516c6b457e73295ec6245354a",
  "target_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
  "changed_paths": [
    {
      "path": "thermal_control/component.heat_exchanger.param.setpoint_trim",
      "change_kind": "set",
      "before_leaf_hash": "afa9ca7b43d8490d25b4cef036e9738993622ec64f516ae17a1f3d945bfb6ac9",
      "after_leaf_hash": "bbefe09cac7b887d684268bb7d078e2341b00831c08949c5f2b2e7ae8b298d38",
      "before_value": 0.35,
      "after_value": 0.42
    }
  ],
  "delta_manifest": {
    "schema_version": 5,
    "manifest_id": "ce975ffc020336f15ee296eb21eb6d3b2ee34595d22576a178db3e56837138c5",
    "base_configuration_id": "70a8b15a84768a8f922387e61866ab183a2c368516c6b457e73295ec6245354a",
    "target_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
    "changed_paths": [
      {
        "path": "thermal_control/component.heat_exchanger.param.setpoint_trim",
        "change_kind": "set",
        "before_leaf_hash": "afa9ca7b43d8490d25b4cef036e9738993622ec64f516ae17a1f3d945bfb6ac9",
        "after_leaf_hash": "bbefe09cac7b887d684268bb7d078e2341b00831c08949c5f2b2e7ae8b298d38",
        "before_value": 0.35,
        "after_value": 0.42
      }
    ],
    "created_at_unix_ms": 1700000060000,
    "actor": "operator-a",
    "reason": "persist trim"
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

`commit_id`, `base_configuration_id`, `target_configuration_id` and
`delta_manifest` are all omitted from a failed commit.

Nested (`RuntimeDeltaPathChange`):

```json
{
  "path": "thermal_control/component.heat_exchanger.param.setpoint_trim",
  "change_kind": "set",
  "before_leaf_hash": "afa9ca7b43d8490d25b4cef036e9738993622ec64f516ae17a1f3d945bfb6ac9",
  "after_leaf_hash": "bbefe09cac7b887d684268bb7d078e2341b00831c08949c5f2b2e7ae8b298d38",
  "before_value": 0.35,
  "after_value": 0.42
}
```

`change_kind` is `set`, `delete` or `metadata`. The two values are carried
alongside the two leaf hashes.

Nested (`RuntimeDeltaManifest`):

```json
{
  "schema_version": 5,
  "manifest_id": "ce975ffc020336f15ee296eb21eb6d3b2ee34595d22576a178db3e56837138c5",
  "base_configuration_id": "70a8b15a84768a8f922387e61866ab183a2c368516c6b457e73295ec6245354a",
  "target_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
  "changed_paths": [
    {
      "path": "thermal_control/component.heat_exchanger.param.setpoint_trim",
      "change_kind": "set",
      "before_leaf_hash": "afa9ca7b43d8490d25b4cef036e9738993622ec64f516ae17a1f3d945bfb6ac9",
      "after_leaf_hash": "bbefe09cac7b887d684268bb7d078e2341b00831c08949c5f2b2e7ae8b298d38",
      "before_value": 0.35,
      "after_value": 0.42
    }
  ],
  "created_at_unix_ms": 1700000060000,
  "actor": "operator-a",
  "reason": "persist trim"
}
```

`manifest_id` is not a counter: the runtime derives it as a sha256 over the
canonical JSON of the schema version, the journal sequence, both configuration
ids, the change set, the actor and the reason, so it is a content address over
the delta this manifest describes rather than the journal sequence `commit_id`
renders.

Semantics:

1. Commit remains full-state semantic at API level.
2. Persistence and sync artifacts are delta-first and include path-granular change manifests.

### 6.6 `get_configuration_identity`

Request (`GetConfigurationIdentityRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5
}
```

Response (`GetConfigurationIdentityResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "identity": {
    "committed_configuration_id": "70a8b15a84768a8f922387e61866ab183a2c368516c6b457e73295ec6245354a",
    "working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
    "diff_hash": "fc83885be7b1319dd4fc51ff2a42d33c3acba5571213ae7494788cc09f56a551",
    "dirty_diff_hash": "011423ae0616e036c738735a6768bda837ff3379dc4b04e10f6467f7147fad33"
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

`model_hash` and `resolve_hash` are common envelope fields and stay top level.
The four configuration identifiers are nested under `identity` and are absent
when the operation failed.

Nested (`RuntimeConfigurationIdentity`):

```json
{
  "committed_configuration_id": "70a8b15a84768a8f922387e61866ab183a2c368516c6b457e73295ec6245354a",
  "working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
  "diff_hash": "fc83885be7b1319dd4fc51ff2a42d33c3acba5571213ae7494788cc09f56a551",
  "dirty_diff_hash": "011423ae0616e036c738735a6768bda837ff3379dc4b04e10f6467f7147fad33"
}
```

`diff_hash` covers the committed overlay against the resolved baseline;
`dirty_diff_hash` covers the dirty overlay on top of that. A session with no
dirty writes has `working_configuration_id` equal to
`committed_configuration_id`.

Scope of the two configuration ids. Both are Merkle roots over **parameter
leaves only** — one leaf per `component.<component_id>.param.<param_key>` path,
carrying that parameter's type, unit, lifecycle and effective value. Requirement
fields (ADR-0057 §D7, readable at
`component.<component_id>.requires.<slot>.<field>`) are deliberately **not**
leaves: the roots exist to identify mutable configuration state, and a
requirement value is fixed at resolve time and cannot be written. They are not
unprotected — they are covered by `resolve_hash`, which `runtime_open`
recomputes and cross-validates on every open, so a snapshot whose delivered
entries were edited never opens. An attestation consumer must therefore treat
the configuration ids as evidence about parameters, and `resolve_hash` as the
evidence about everything the resolve delivered.

### 6.7 `set_auto_reset_policy` and `get_auto_reset_policy`

Both operations carry the policy as a nested `auto_reset_policy` object rather
than as loose fields on the envelope.

Request (`SetAutoResetPolicyRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "auto_reset_policy": {
    "enabled": true,
    "default_timeout_ms": 60000,
    "per_path_overrides": {
      "thermal_control/component.heat_exchanger.param.setpoint_trim": {
        "enabled": false
      }
    },
    "policy_revision": 3
  }
}
```

Response (`SetAutoResetPolicyResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "auto_reset_policy": {
    "enabled": true,
    "default_timeout_ms": 60000,
    "per_path_overrides": {
      "thermal_control/component.heat_exchanger.param.setpoint_trim": {
        "enabled": false
      }
    },
    "policy_revision": 4
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

Request (`GetAutoResetPolicyRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5
}
```

Response (`GetAutoResetPolicyResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "auto_reset_policy": {
    "enabled": true,
    "default_timeout_ms": 60000,
    "per_path_overrides": {
      "thermal_control/component.heat_exchanger.param.setpoint_trim": {
        "enabled": false
      }
    },
    "policy_revision": 4
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

Nested (`AutoResetPolicy`):

```json
{
  "enabled": true,
  "default_timeout_ms": 60000,
  "per_path_overrides": {
    "thermal_control/component.heat_exchanger.param.setpoint_trim": {
      "enabled": false
    }
  },
  "policy_revision": 3
}
```

`per_path_overrides` is a JSON **object** keyed by path, not an array. A key may
be given in either the scope-qualified form shown above or as a bare
`component.<component_id>.param.<param_key>`; the runtime looks up the
scope-qualified key first and falls back to the bare one. `enabled` is the
global default and `default_timeout_ms` the global timeout; an override supplies
either or both for one path.

Nested (`AutoResetPathPolicy`):

```json
{
  "enabled": false,
  "timeout_ms": 30000
}
```

Both fields are optional. An omitted field falls back to the corresponding
global policy field, so `{ "timeout_ms": 30000 }` shortens one path's timeout
without changing whether auto-reset is enabled for it.

### 6.8 Sync Operations

`check_for_updates`, `pull_updates` and `get_sync_status` each return an
optional `sync_status` object of type `RuntimeSyncStatus`, omitted entirely when
the operation reports no sync state. The four sync fields nest under that
object; none of them is a top-level response field.

Request (`CheckForUpdatesRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "backend_connected": true,
  "pending_update_summary": "3 parameters changed upstream"
}
```

`backend_connected` defaults to `true` and is always serialized.
`pending_update_summary` is optional and carries the caller's own description of
what it believes is waiting upstream.

Response (`CheckForUpdatesResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "sync_status": {
    "sync_state": "idle",
    "last_successful_sync_unix_ms": 1700000030000,
    "pending_update_summary": "3 parameters changed upstream",
    "sync_diagnostics": []
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

Request (`PullUpdatesRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "actor": "sync-agent",
  "reason": "scheduled pull",
  "backend_connected": true,
  "source": "backend",
  "writes": [
    {
      "path": "component.heat_exchanger.param.setpoint_trim",
      "value": 0.5,
      "before_leaf_hash": "bbefe09cac7b887d684268bb7d078e2341b00831c08949c5f2b2e7ae8b298d38",
      "after_leaf_hash": "7d7bb9052b555f652a4ca15530ebba920a2eeed495904ac7bea6bd004cfe8fbc"
    }
  ],
  "base_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
  "full_snapshot": false,
  "pending_update_summary": "1 parameter changed upstream",
  "target_configuration_id": "5402d2d322906b37f76bb1e8f97d14e7225a026827bd1c2f456781c24a70ab23"
}
```

`actor` is required. `source` is `backend` or `direct_push`, and `full_snapshot`
separates a bootstrap payload from an incremental one: an incremental update
must carry both `base_configuration_id` and `target_configuration_id`, or it is
rejected with `E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED`. `source`, `writes` and
`full_snapshot` all have defaults but are always serialized.

Four fields on this request are sha-256 digests, and the runtime checks their
shape before it checks anything they say: `base_configuration_id`,
`target_configuration_id`, and each write's `before_leaf_hash` and
`after_leaf_hash`. A value that is present must be exactly 64 lowercase ASCII
hex characters, whatever `full_snapshot` says, or the request is rejected with
`E_RUNTIME_SYNC_PAYLOAD_INVALID` and a message naming the field, such as
`request.base_configuration_id must be a 64-char lowercase sha256 hex string`.
All four are optional and the check applies only to a value that is there:
surrounding whitespace is trimmed and a blank string counts as absent, so
omitting a configuration id on an incremental request reports the missing field
rather than a malformed one. These ids are not caller-chosen. A base id comes from
`get_configuration_identity` (section 6.6); a target id and both leaf hashes
come from the backend or direct-push delta manifest that produced the update.

Nested (`PullUpdateWrite`):

```json
{
  "path": "component.heat_exchanger.param.setpoint_trim",
  "value": 0.5,
  "before_leaf_hash": "bbefe09cac7b887d684268bb7d078e2341b00831c08949c5f2b2e7ae8b298d38",
  "after_leaf_hash": "7d7bb9052b555f652a4ca15530ebba920a2eeed495904ac7bea6bd004cfe8fbc"
}
```

Both leaf hashes are optional on the wire, but an incremental write that omits
`before_leaf_hash`, or carries one that no longer matches the parameter's
committed value, is rejected with `E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH`.

Every hash the caller supplies is verified against one the runtime computes for
itself, so these fields are preconditions rather than annotations. On an
incremental request `base_configuration_id` must equal the runtime's current
committed configuration id, or the update is rejected with
`E_RUNTIME_SYNC_BASE_MISMATCH`; a bootstrap request (`full_snapshot: true`)
skips that comparison but still has the id it supplies checked for shape. Each
write's `after_leaf_hash` must equal the leaf hash the runtime derives from the
value carried in that same write, and `target_configuration_id` must equal the
configuration id the runtime computes after applying the whole batch — not the
id the update was built against, and not one the runtime merely records. Either
mismatch is rejected with `E_RUNTIME_SYNC_TARGET_HASH_MISMATCH`. A request that
carries no writes applies nothing, so its target id is compared against the
current committed id. Comparison is exact string equality against digests the
runtime renders in lowercase, which is why the format check above requires
lowercase: an uppercase id is refused on shape rather than reported as a
divergence it never was. Each of these checks reports the first failure and
stops, and a rejected `pull_updates` returns no `runtime_snapshot`: a caller
that threads the snapshot forward from one call to the next therefore applies
none of the batch.

Response (`PullUpdatesResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "applied_paths": [
    "thermal_control/component.heat_exchanger.param.setpoint_trim"
  ],
  "conflict_paths": [
    "thermal_control/component.heat_exchanger.param.setpoint_trim"
  ],
  "base_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
  "target_configuration_id": "5402d2d322906b37f76bb1e8f97d14e7225a026827bd1c2f456781c24a70ab23",
  "sync_status": {
    "sync_state": "idle",
    "last_successful_sync_unix_ms": 1700000090000,
    "pending_update_summary": "1 parameter changed upstream",
    "sync_diagnostics": [
      {
        "code": "E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN",
        "severity": "warning",
        "message": "Upstream values overrode 1 locally dirty changed path(s)",
        "entity_path": "request.writes",
        "hint": "Review conflict_paths for commissioning/reconciliation follow-up"
      }
    ]
  },
  "audit_event_id": "audit-000000000000001a",
  "error_count": 0,
  "warning_count": 1,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [
      {
        "code": "E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN",
        "severity": "warning",
        "message": "Upstream values overrode 1 locally dirty changed path(s)",
        "entity_path": "request.writes",
        "hint": "Review conflict_paths for commissioning/reconciliation follow-up"
      }
    ],
    "error_count": 0,
    "warning_count": 1
  }
}
```

`applied_paths` names every path the batch wrote, scope-qualified.
`conflict_paths` is the subset of those where a local dirty override existed and
the upstream value won; a conflicting path is applied like any other, so it
appears in both lists. A conflicting apply raises exactly one aggregate warning,
never one per path: a single `E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN` diagnostic
whose message counts the overridden paths and whose `entity_path` is
`request.writes`. That one warning is reported in two places, once in
`diagnostics.diagnostics` and once in `sync_status.sync_diagnostics`, so
`warning_count` stays `1` however many paths conflicted. It is a warning rather
than an error, so the update still succeeds. `audit_event_id` names the audit
record the apply wrote. The two configuration ids and `sync_status` are optional
and omitted when the operation has nothing to report for them.

Request (`GetSyncStatusRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5
}
```

Response (`GetSyncStatusResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "sync_status": {
    "sync_state": "offline",
    "last_successful_sync_unix_ms": 1700000090000,
    "sync_diagnostics": []
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

Nested (`RuntimeSyncStatus`):

```json
{
  "sync_state": "idle",
  "last_successful_sync_unix_ms": 1700000090000,
  "pending_update_summary": "3 parameters changed upstream",
  "sync_diagnostics": []
}
```

`sync_state` is one of `idle`, `checking`, `pulling`, `applying`, `error` or
`offline`, and defaults to `idle`. `last_successful_sync_unix_ms` is Unix
milliseconds, not a formatted UTC string, and is omitted until a sync has
succeeded. `pending_update_summary` is optional. `sync_diagnostics` is always
present, empty when there is nothing to report.

### 6.9 Event Subscription

Request (`SubscribeEventsRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "from_sequence": 12,
  "max_events": 256,
  "event_kinds": [
    "parameter_changed",
    "dirty_state_changed"
  ]
}
```

`from_sequence` is the cursor, `max_events` caps the batch and defaults to
`256`, and an empty `event_kinds` selects every kind. All three have defaults
but are always serialized.

Response (`SubscribeEventsResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "from_sequence": 12,
  "next_sequence": 19,
  "dropped_events": 0,
  "events": [
    {
      "event_id": "evt-000000000000000d",
      "sequence": 13,
      "event_kind": "parameter_changed",
      "scope": "all",
      "timestamp_unix_ms": 1700000045000,
      "actor": "operator-a",
      "reason": "commissioning trim",
      "old_value_hash": "a976726b1d1bda3e1f271287f3aa7229ca3204ff8bf9cb2b581c610b9c811bef",
      "new_value_hash": "16ad1c0bd6e26646912d571c740893247f61468655d52878989e9d858710655b",
      "payload": {
        "payload_kind": "parameter_changed",
        "scope_root": "thermal_control",
        "path": "component.heat_exchanger.param.setpoint_trim",
        "generation": 4
      }
    }
  ],
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

The response echoes the `from_sequence` it was asked for, which is exclusive:
the batch starts at the first event above it. The cursor to send on the
following call is the highest `sequence` among the returned `events` — `13`
above, not `19` — and when `events` is empty the caller keeps the cursor it
already had. It is not `next_sequence`: that field reports where the stream head
is, the sequence the runtime will assign to the next event it emits, and it is
unaffected by both the `max_events` cap and the `event_kinds` filter that decide
what this batch actually carried. Sending `next_sequence` back as the next
`from_sequence` therefore skips every event the call did not return — the five
between `13` and `19` above, which the request's `event_kinds` filtered out.
Comparing the two tells a consumer whether it has drained the stream.
`dropped_events` is the running count of events the bounded buffer has evicted
over the session, so a consumer can tell a gap from an idle stream.

Nested (`RuntimeEvent`):

```json
{
  "event_id": "evt-000000000000000d",
  "sequence": 13,
  "event_kind": "parameter_changed",
  "scope": "all",
  "timestamp_unix_ms": 1700000045000,
  "actor": "operator-a",
  "reason": "commissioning trim",
  "old_value_hash": "a976726b1d1bda3e1f271287f3aa7229ca3204ff8bf9cb2b581c610b9c811bef",
  "new_value_hash": "16ad1c0bd6e26646912d571c740893247f61468655d52878989e9d858710655b",
  "payload": {
    "payload_kind": "parameter_changed",
    "scope_root": "thermal_control",
    "path": "component.heat_exchanger.param.setpoint_trim",
    "generation": 4
  }
}
```

`event_id` is derived from `sequence`, not independent of it: the runtime
formats it as `evt-` followed by the sequence as sixteen lowercase hex digits,
so sequence `13` carries `evt-000000000000000d`. `timestamp_unix_ms` is Unix
milliseconds, not a formatted UTC string. `actor`, `reason`, `old_value_hash`
and `new_value_hash` are optional; `payload` is required. An affected path is
reported inside `payload`, never at the top level of the event.

Nested (`RuntimeEventPayload`), the `parameter_changed` variant:

```json
{
  "payload_kind": "parameter_changed",
  "scope_root": "thermal_control",
  "path": "component.heat_exchanger.param.setpoint_trim",
  "generation": 4
}
```

The payload is a tagged union: `payload_kind` selects the variant, and the
remaining fields belong to that variant alone, so the object's shape differs per
kind. The variants are `runtime_opened`, `parameter_changed`,
`dirty_state_changed`, `reset_applied`, `rollback_applied`, `commit_applied`,
`sync_state_changed`, `sync_conflict_detected`, `sync_conflict_resolved`,
`sync_apply_completed` and `override_escalated`, matching the event kinds of the
same names. One variant is shown here; a consumer must switch on `payload_kind`
rather than assume the fields above.

### 6.10 `push_audit_events`

Request (`PushAuditEventsRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "backend_connected": true,
  "max_events": 256
}
```

`max_events` caps the batch and defaults to `256`.

Response (`PushAuditEventsResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "pushed_event_ids": [
    "audit-000000000000000b",
    "audit-000000000000000c"
  ],
  "pushed_count": 2,
  "pending_count": 0,
  "last_uploaded_sequence": 12,
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

`pushed_event_ids` holds the `event_id` of every audit event this call uploaded,
in sequence order, and `last_uploaded_sequence` is the sequence of the last of
them. The two agree by construction: the ids above end at
`audit-000000000000000c`, which is sequence `12`.

Semantics:

1. Audit events are persisted locally before push attempts.
2. Offline push attempts are non-destructive and deterministic.
3. Online push advances an idempotent upload cursor (`audit_uploaded_sequence`) only for acknowledged sequence prefix.

### 6.11 `export_pending_sync_bundle`

Request (`ExportPendingSyncBundleRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "max_audit_events": 256
}
```

`max_audit_events` caps how many pending audit events the bundle carries and
defaults to `256`.

Response (`ExportPendingSyncBundleResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "resolve_hash": "b76eb0ca2e135c25c251f7ef58efdd45a63272120bb9b3c391f7c261b621679c",
  "scope": "all",
  "bundle": {
    "schema_version": 5,
    "bundle_id": "offline-sync-84fa2f1ca5e51e73",
    "generated_at_unix_ms": 1700000120000,
    "committed_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
    "working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
    "dirty_paths": [
      "thermal_control/component.heat_exchanger.param.setpoint_trim"
    ],
    "audit_uploaded_sequence": 12,
    "pending_audit_count": 1,
    "pending_audit_events": [
      {
        "event_id": "audit-000000000000000d",
        "sequence": 13,
        "event_kind": "write",
        "scope": "all",
        "timestamp_unix_ms": 1700000110000,
        "actor": "operator-a",
        "reason": "commissioning trim",
        "committed_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
        "working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
        "changed_paths": [
          "thermal_control/component.heat_exchanger.param.setpoint_trim"
        ]
      }
    ],
    "sync_status": {
      "sync_state": "offline",
      "sync_diagnostics": []
    }
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

`bundle` is optional and omitted entirely from a failed export.

Nested (`OfflineReconciliationBundle`):

```json
{
  "schema_version": 5,
  "bundle_id": "offline-sync-84fa2f1ca5e51e73",
  "generated_at_unix_ms": 1700000120000,
  "committed_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
  "working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
  "dirty_paths": [
    "thermal_control/component.heat_exchanger.param.setpoint_trim"
  ],
  "audit_uploaded_sequence": 12,
  "pending_audit_count": 1,
  "pending_audit_events": [
    {
      "event_id": "audit-000000000000000d",
      "sequence": 13,
      "event_kind": "write",
      "scope": "all",
      "timestamp_unix_ms": 1700000110000,
      "actor": "operator-a",
      "reason": "commissioning trim",
      "committed_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
      "working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
      "changed_paths": [
        "thermal_control/component.heat_exchanger.param.setpoint_trim"
      ]
    }
  ],
  "sync_status": {
    "sync_state": "offline",
    "sync_diagnostics": []
  }
}
```

Every field of the bundle is required. `bundle_id` is not a counter and carries
no sequence: the runtime derives it as `offline-sync-` followed by the first
sixteen hex characters of a hash over the bundle's own contents, so it changes
whenever the pending set does and two exports of the same pending set agree. The
bundle carries its own `schema_version` and a `generated_at_unix_ms` stamp,
`audit_uploaded_sequence` is the idempotent upload cursor section 6.10 advances,
and `sync_status` is a required member here rather than the optional one the
three sync results return.

Nested (`RuntimeAuditEvent`):

```json
{
  "event_id": "audit-000000000000000d",
  "sequence": 13,
  "event_kind": "write",
  "scope": "all",
  "timestamp_unix_ms": 1700000110000,
  "actor": "operator-a",
  "reason": "commissioning trim",
  "committed_configuration_id": "6acff53c25aff43c1448708d76e2fa712fb2a1b9a9f2c151f95fd2de68b38b0e",
  "working_configuration_id": "b45e7a4ec3041ce1f9dec828ba7444906eed06dcb07cd24ff6a3ba2dfa4c9565",
  "changed_paths": [
    "thermal_control/component.heat_exchanger.param.setpoint_trim"
  ]
}
```

`event_id` is derived from `sequence` the same way a `RuntimeEvent`'s is, under a
different prefix: `audit-` followed by the sequence as sixteen lowercase hex
digits, so sequence `13` carries `audit-000000000000000d`. `event_kind` is
`write`, `reset`, `commit`, `sync_apply`, `direct_push` or `escalation`. `actor`
is required here, unlike the optional `actor` a `RuntimeEvent` carries. `base_configuration_id` and `target_configuration_id`
are optional and present only on the kinds that move between configurations.
`changed_paths` is always serialized, empty when the event changed nothing.

Semantics:

1. Export is read-only and deterministic for equivalent snapshot state.
2. Bundle payload is suitable for later manual backend reconciliation.
3. Direct push and backend sync use the same lineage/audit data model, specified in section 11.

### 6.12 Constraint enforcement on writes

This subsection carries no fenced example because it introduces no payload type
of its own. It states enforcement semantics over payloads already pinned above:
the write request and its result by 6.1 — including the `unsat_core` object a
constraint rejection adds to `SetParametersAtomicallyResult` — and the commit by
6.5.

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
7. A parameter is a facet's handle only when the model declares
   `facet: <name>` on it; the compiler enforces one handle per facet, so two
   parameters can never map to one facet. A write to a parameter without a
   declared binding is never constraint-checked.

### 6.13 `explain_rejection`

`explain-rejection` answers one question: why would setting this parameter to
this value be refused? It is a read-side query (ADR-0031 D1). It evaluates the
candidate against the session the request carries, returns no
`runtime_snapshot`, and leaves the session exactly as it was. Section 5 records
why it sits outside both operation lists.

The request names its subject in the runtime's own vocabulary rather than the
solver's: `path` is the `component.<component_id>.param.<param_key>` write path
`set_parameter` takes, and `value` is the candidate. The parameter's declared
facet is the facet the solver explains, and a string `value` is the **option**
it explains. A non-string `value` names no modeled option, and a parameter that
declares no facet names no modeled facet; both come back as a command error with
no core rather than as an explanation.

Request (`RuntimeExplainRejectionRequest`), `runtime_snapshot` elided:

```json
{
  "schema_version": 5,
  "path": "component.coolant_loop.param.pump_mode",
  "value": "variable_speed"
}
```

Response (`RuntimeExplainRejectionResult`):

```json
{
  "schema_version": 5,
  "status": "ok",
  "model_hash": "256e19d9c42eb7eace43a05232d78d8c41ea4a2a38e2c5c8a58a347a0bf556a8",
  "scope": "all",
  "path": "component.coolant_loop.param.pump_mode",
  "value": "variable_speed",
  "rejection": {
    "code": "E_SELECTION_CONFLICT",
    "message": "Setting 'component.coolant_loop.param.pump_mode' to 'variable_speed' conflicts with the current configuration",
    "blocking_choices": {
      "drive_topology": "direct_online"
    },
    "hint": "Choose a value for facet 'pump_mode' consistent with the current selection",
    "unsat_core": {
      "rejected": {
        "facet": "pump_mode",
        "option": "variable_speed"
      },
      "conflicting_constraints": [
        {
          "kind": "selection",
          "facets": [
            {
              "facet": "drive_topology",
              "option": "direct_online"
            }
          ],
          "summary": "blocked by your earlier choice: drive_topology.direct_online"
        },
        {
          "kind": "model_rule",
          "facets": [
            {
              "facet": "drive_topology",
              "option": "direct_online"
            },
            {
              "facet": "pump_mode",
              "option": "variable_speed"
            }
          ],
          "summary": "pump_mode != 'variable_speed' || drive_topology == 'inverter'",
          "constraint_id": "variable_speed_requires_inverter"
        }
      ],
      "minimal": true,
      "note": "one minimal explanation; other minimal cores may exist"
    }
  },
  "error_count": 0,
  "warning_count": 0,
  "diagnostics": {
    "schema_version": 5,
    "diagnostics": [],
    "error_count": 0,
    "warning_count": 0
  }
}
```

**A rejection is a successful response.** `status` is `ok` and the exit code is
`0`, even though the answer is "this value would be refused". Asking why an
option is rejected and being told why is the success path, and ADR-0031 D2
states the rule in those words: *a rejection is not an error*. Exit `2` with
`status = error` is reserved for the cases where no explanation could be
computed at all — the path names no modeled facet or the value names no modeled
option, both division-of-labor cases the compiler owns without the solver
(ADR-0030 D5), or the solver faulted (ADR-0031 D4). `error_count`,
`warning_count` and `diagnostics[]` are therefore empty here: a successful query
is not a diagnostic condition.

**This is the one response envelope that does not carry `resolve_hash`**, and
the omission is by design rather than an oversight: the response field list
ADR-0031 D2 freezes does not include it, and section 4's callout names this
envelope for exactly that reason. Every other field section 4 lists is present.
An integrator writing one code path over every runtime response has to treat
`resolve_hash` as absent on this one.

`path` and `value` are echoed from the request, so the response is
self-describing in the caller's own vocabulary; `model_hash` and `scope` are
copied from the session snapshot.

`rejection` carries the explanation. `code` and `message` are always present.
So is `blocking_choices`, which is serialized even when it is empty — a caller
may read it unconditionally. It maps each facet named by a `selection` clause of
the core to the option already chosen for that facet, and it is derived from
those clauses alone. `hint` is advisory and omitted when there is nothing useful
to say.

`unsat_core` is present exactly when `code` is `E_SELECTION_CONFLICT` or
`E_SELECTION_UNSATISFIABLE` (ADR-0031 D3) — the genuine "this selection
contradicts the model" rejections, which are the ones minimal-core extraction
can explain. It is omitted for the rejections the compiler decides without the
solver, and for every command error.

Nested (`UnsatCore`):

```json
{
  "rejected": {
    "facet": "pump_mode",
    "option": "variable_speed"
  },
  "conflicting_constraints": [
    {
      "kind": "selection",
      "facets": [
        {
          "facet": "drive_topology",
          "option": "direct_online"
        }
      ],
      "summary": "blocked by your earlier choice: drive_topology.direct_online"
    },
    {
      "kind": "model_rule",
      "facets": [
        {
          "facet": "drive_topology",
          "option": "direct_online"
        },
        {
          "facet": "pump_mode",
          "option": "variable_speed"
        }
      ],
      "summary": "pump_mode != 'variable_speed' || drive_topology == 'inverter'",
      "constraint_id": "variable_speed_requires_inverter"
    }
  ],
  "minimal": true,
  "note": "one minimal explanation; other minimal cores may exist"
}
```

`rejected` echoes the `{facet, option}` pair being explained.
`conflicting_constraints` is the labeled minimal unsatisfiable subset: the
smallest set of constraints that, together with `rejected`, admit no
configuration at all. `kind` tells a `selection` — a choice the session already
made — apart from a `model_rule` baked into the compiled model. `facets` names
every participant as a labeled `{facet, option}` pair and never carries a raw
solver variable index, which ADR-0031 D3 states as a schema invariant rather
than a convention. `summary` is a one-line human gloss, advisory text and not a
parsed field: for a `selection` it reads
`blocked by your earlier choice: <facet>.<option>`, and for a `model_rule` it is
the authored condition text of the constraint `constraint_id` names.

`constraint_id` is the field a machine consumer reads to tie a conflict back to
declared policy: it names the authored `constraints:` entry a `model_rule`
clause is attributed to. It is omitted on a `selection`, because a prior choice
is not a declared constraint, and on a `model_rule` that no declared constraint
accounts for. That second case says the model over-constrains the combination
rather than that the caller broke a policy, and `summary` then reads `the model
is over-constrained here; no declared constraint accounts for this conflict`.

`minimal` is `true` on every core the runtime emits today. It exists as a field
so a future non-minimal fast path can report `false` without a schema break.
`note` is a fixed advisory string, `one minimal explanation; other minimal cores
may exist`, and it is load-bearing: extraction returns *a* minimal core, not
*the* canonical one, so two runs over the same inputs may report two
equally-minimal cores. The schema deliberately promises nothing about
determinism across runs. A caller that needs stable diffing keys on the set of
`conflicting_constraints` and reads it as one witness among possibly several.

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
3. A response carries the common envelope of section 4 plus the fields of the
   operation that produced it, and nothing else. That envelope is the same for
   both families, so no top-level field distinguishes a v1 response from a v2
   one; the two differ only in the operation-specific fields below it. No field
   is gated on a requested version and no request value can add one to a v1
   response — `unsat_core` and `runtime_snapshot` are carried by v1 results as
   well as v2 ones, so "v2-only field" names nothing real.
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
   version (value `5`). This field does not appear in the serialized entry; it is
   injected into the canonical payload before hashing. It is the same value the
   request and response envelopes carry (section 2), applied to the payload
   rather than sent on the wire. The pin ties an entry's address to the schema
   version in force when it was computed; entries already addressed under an
   earlier version keep their addresses.
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
{"schema_version":5,"state":{"model_version":"m1","selection_version":"s1","override_layer":"o1"},"actor":"operator-a","reason":"initial state","timestamp_unix_ms":1700000000000,"intent":"experimental","parent_entry_id":null}
```

The SHA-256 of exactly those bytes is its `entry_id`:

```
d49885b4ac479184c8319314390d0517e0b60f8e8a2215ee68ff67cc34f3ea1d
```

A child entry that pins `model_version = "m2"` (other state axes unchanged),
`actor = "operator-a"`, no `reason`, `timestamp_unix_ms = 1700000005000`,
`intent = compensating`, and `parent_entry_id` set to the root's `entry_id` above
has canonical payload:

```json
{"schema_version":5,"state":{"model_version":"m2","selection_version":"s1","override_layer":"o1"},"actor":"operator-a","reason":null,"timestamp_unix_ms":1700000005000,"intent":"compensating","parent_entry_id":"d49885b4ac479184c8319314390d0517e0b60f8e8a2215ee68ff67cc34f3ea1d"}
```

and `entry_id`:

```
a29574581c3d167e9542c031546eb3ed621cdf4a596d93b05ae19f835a8d631d
```

The child illustrates both optional-field rules: `reason` serializes as `null`
(absent) and `parent_entry_id` carries the root's address, both inside the hashed
bytes.
