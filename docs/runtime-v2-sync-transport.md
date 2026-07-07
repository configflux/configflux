# Runtime v2 Sync and Telemetry Transport Contract (MQTT First)

Status: frozen
Date: 2026-02-14
Depends on: `docs/runtime-v2-contract.md`, `docs/runtime-v2-hash-delta-protocol.md`

## 1. Scope

This document defines the pluggable sync transport contract with MQTT as the first implementation.

Covered channels:

1. Update check and pull
2. Delta apply acknowledgments
3. Telemetry upload
4. Audit event upload
5. Sync status heartbeat

## 2. Transport Abstraction

Runtime core transport interface:

1. `connect(session_config) -> Result`
2. `publish(channel, envelope) -> Result`
3. `request(channel, envelope, timeout) -> response`
4. `subscribe(channel, handler) -> subscription`
5. `disconnect()`

Transport-neutral requirements:

1. Deterministic envelope encoding
2. Idempotency support
3. Retry/backoff policy hooks
4. Bounded queueing with backpressure signals

## 3. Common Sync Envelope Fields

All transport envelopes include:

1. `schema_version`
2. `message_id`
3. `correlation_id`
4. `device_id`
5. `product_id`
6. `timestamp_utc`
7. `channel_kind`
8. `payload`

Idempotency key:

1. `idempotency_key = sha256(channel_kind + message_id + base_configuration_id? + target_configuration_id?)`

## 4. MQTT Topic Classes

Topic namespace:

1. `cfg/<product_id>/<device_id>/updates/announce`
2. `cfg/<product_id>/<device_id>/updates/pull/request`
3. `cfg/<product_id>/<device_id>/updates/pull/response`
4. `cfg/<product_id>/<device_id>/updates/apply/ack`
5. `cfg/<product_id>/<device_id>/telemetry/batch`
6. `cfg/<product_id>/<device_id>/audit/events`
7. `cfg/<product_id>/<device_id>/status/heartbeat`

MQTT requirements:

1. QoS profile is configurable per topic class.
2. Session clean-start behavior is explicit and deterministic.
3. Reconnect resumes subscriptions and outstanding correlation tracking.

## 5. Retry and Backoff Policy

Default behavior:

1. Exponential backoff with jitter for connect/publish/request retries.
2. Per-channel retry budgets.
3. Retry-disabled mode for deterministic test fixtures.

Deterministic failure mapping:

1. Timeout -> `E_RUNTIME_SYNC_TIMEOUT`
2. Disconnect -> `E_RUNTIME_SYNC_TRANSPORT_DISCONNECTED`
3. Auth failure -> `E_RUNTIME_SECURITY_AUTHN_*`
4. Invalid payload -> `E_RUNTIME_SYNC_PAYLOAD_INVALID`

## 6. Online and Offline Behavior

Online mode:

1. Perform periodic update checks.
2. Pull and apply updates under policy.
3. Flush telemetry/audit queues.

Offline mode:

1. Continue local runtime operations.
2. Buffer telemetry and audit to local sinks.
3. Mark sync state with deterministic stale markers.
4. Resume uploads from persisted queues on reconnect.

## 7. Upstream-Authoritative Apply Policy

Sync apply contract:

1. Payloads include `base_configuration_id`, `target_configuration_id`, and `changed_paths`.
2. Runtime validates base and before-hash per changed path.
3. Upstream value wins for changed keys, even if local dirty exists on those keys.
4. Keys not listed in `changed_paths` are untouched.

Conflict behavior:

1. Emit conflict event before apply decision.
2. Apply upstream changed keys atomically.
3. Emit post-apply event with conflict resolution summary.

## 8. Telemetry and Audit Transport Rules

1. Telemetry batches are ordered by local event sequence.
2. Audit events are immutable and include actor and lineage fields.
3. Upload ack includes accepted range and checksum.
4. Failed uploads remain replayable and idempotent.

## 9. Security Binding

1. Transport sessions use TLS per `docs/runtime-v2-security-envelope.md`.
2. Auth identity metadata is attached to transport session context.
3. Secret-bearing fields are never emitted in diagnostics or plain logs.

## 10. Extensibility Rules

1. Additional transports (REST, gRPC, custom fieldbus) must implement the same interface semantics.
2. Runtime-core sync logic is transport-neutral and cannot depend on MQTT-specific types.
3. New transport adapters must preserve envelope schema and deterministic diagnostics.
