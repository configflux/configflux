# Runtime v2 Security Envelope and External Auth Integration Points

Status: frozen
Date: 2026-02-14

## 1. Scope

This document defines Runtime v2 security boundaries, transport requirements, auth integration hooks, and failure behavior.

In scope:

1. TLS requirements for remote channels
2. External authn/authz integration contracts
3. Actor attribution for audit events
4. Deterministic, non-leaky diagnostics behavior

Out of scope:

1. Vault provider implementation
2. TPM provider implementation
3. Enterprise policy engine implementation

## 2. Trust Boundaries

Primary boundaries:

1. Local runtime core process boundary
2. Local SDK/client boundary (C++, ROS2, tooling)
3. Remote backend transport boundary
4. Persistence boundary (local disk)

Assumptions:

1. Runtime does not trust network origin by default.
2. Runtime trusts only validated envelopes and explicit auth decisions.
3. Runtime disk path permissions are enforced by host OS.

## 3. TLS and Channel Requirements

Remote sync and telemetry channels must:

1. Use TLS 1.3 or newer (TLS 1.2 only by explicit compatibility mode)
2. Validate peer certificates against configured trust roots
3. Support mTLS where deployment requires device identity
4. Enforce hostname or endpoint identity validation

Failure behavior:

1. TLS validation failures fail closed.
2. Runtime transitions to `sync_state = error|offline`.
3. Diagnostics use `E_RUNTIME_SECURITY_*` without exposing secret material.

## 4. External Auth Integration Contracts

Runtime accepts external decision providers through stable interfaces:

1. `AuthnProvider::authenticate(request_context) -> identity | denial`
2. `AuthzProvider::authorize(identity, action, resource) -> allow | deny`
3. `AuditContextProvider::decorate(identity) -> actor_metadata`

Required request context fields:

1. `request_id`
2. `operation`
3. `scope`
4. `path_set`
5. `transport_origin`
6. `client_cert_fingerprint?`

## 5. Actor and Audit Requirements

Every mutating operation (`write`, `rollback`, `commit`, `sync_apply`, `direct_push`) must include:

1. `actor_id`
2. `auth_source` (`local|external|system`)
3. `reason?`
4. `timestamp_utc`
5. `base_configuration_id`
6. `target_configuration_id` (when applicable)

If actor data is unavailable:

1. Operation may proceed only for explicitly allowed system paths.
2. Runtime uses deterministic placeholder actor (`system/unknown`) and emits warning diagnostics.

## 6. Key Material Ownership

1. Runtime reads trust roots and client credentials from configured files or injected handles.
2. Runtime does not generate or rotate enterprise root keys.
3. Runtime may rotate local session credentials only through externally provided mechanism.
4. Secret values are never logged in plaintext.

## 7. Fail-Closed Behavior Matrix

| Condition | Runtime Behavior | Diagnostic Family |
| --- | --- | --- |
| Authn provider unavailable | Reject remote mutating requests | `E_RUNTIME_SECURITY_AUTHN_UNAVAILABLE` |
| Authz decision = deny | Reject operation | `E_RUNTIME_SECURITY_AUTHZ_DENIED` |
| TLS validation failure | Reject connection/session | `E_RUNTIME_SECURITY_TLS_INVALID` |
| Missing actor on required mutation | Reject unless explicitly system-allowed | `E_RUNTIME_SECURITY_ACTOR_REQUIRED` |
| Audit sink unavailable | Persist locally, mark deferred upload | `E_RUNTIME_TELEMETRY_*` |

## 8. Non-Leaky Diagnostic Policy

Diagnostics and stderr output must not contain:

1. Secrets
2. Private key material
3. Full tokens
4. Raw request bodies for sensitive operations

Allowed diagnostic context:

1. Stable error code
2. Operation name
3. Path summary (bounded length)
4. Correlation or request ID
5. High-level cause category

## 9. Security Diagnostic Families

Runtime v2 reserves and uses:

1. `E_RUNTIME_SECURITY_AUTHN_*`
2. `E_RUNTIME_SECURITY_AUTHZ_*`
3. `E_RUNTIME_SECURITY_TLS_*`
4. `E_RUNTIME_SECURITY_KEYMATERIAL_*`
5. `E_RUNTIME_SECURITY_POLICY_*`

All codes are deterministic and stable across retries for equivalent request context.
