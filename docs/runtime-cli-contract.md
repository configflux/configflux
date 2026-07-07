# Runtime CLI Product Contract v1 (Frozen)

Status: frozen
Date: 2026-02-14

## 1. Scope
This document freezes the runtime v1 binary command surface over
`compiler::runtime_api` for:
1. `runtime_open`
2. `get_scope_metadata`
3. `list_parameters`
4. `get_parameter`
5. `set_parameter`

This contract covers request/response envelopes, deterministic JSON transport,
exit-code behavior, and security invariants.

Runtime v2 command additions are frozen in `docs/runtime-v2-contract.md`.
v1 commands remain backward compatible.

## 2. Commands (`configflux-runtime`)
Runtime v1 commands:
1. `runtime-open`
2. `get-scope-metadata`
3. `list-parameters`
4. `get-parameter`
5. `set-parameter`

Runtime v2 command additions (see `docs/runtime-v2-contract.md`):
1. `set-parameters-atomically`
2. `list-dirty-parameters`
3. `get-dirty-metadata`
4. `rollback-dirty`
5. `commit-configuration`
6. `get-configuration-identity`
7. `set-auto-reset-policy`
8. `get-auto-reset-policy`
9. `check-for-updates`
10. `pull-updates`
11. `get-sync-status`
12. `subscribe-events`
13. `push-audit-events`
14. `export-pending-sync-bundle`

Command-to-API mapping is 1:1:
1. `runtime-open` -> `runtime_open(RuntimeOpenRequest)`, plus the ADR-0030 D2
   `.ccm` usability precondition layered by the runtime CLI (the compiler may
   not import `solver`, ADR-0003 §2)
2. `get-scope-metadata` -> `get_scope_metadata(GetScopeMetadataRequest)`
3. `list-parameters` -> `list_parameters(ListParametersRequest)`
4. `get-parameter` -> `get_parameter(GetParameterRequest)`
5. `set-parameter` -> `set_parameter(SetParameterRequest)`, preceded by the
   solver option-validity pre-check (fail closed on a solver fault, ADR-0030 D4)

## 3. I/O Modes
Default mode:
1. request JSON from `stdin`
2. response JSON to `stdout`

File mode:
1. request JSON from `--request-file <path>`
2. response JSON to `--response-file <path>`

Deterministic transport rules:
1. identical request bytes produce byte-stable response bytes for read commands
2. no nondeterministic metadata is injected by CLI transport
3. response payloads are serialized as one JSON object + trailing newline
4. request payload size is bounded to `8 MiB`

## 4. Exit Codes
1. `0` -> command result `status = ok`
2. `2` -> command result `status = error`
3. `1` -> transport/CLI failure (argument misuse, I/O error, malformed JSON, oversized input)

## 5. Envelope Policy
Requests and responses are frozen to existing runtime API structs in `compiler`:
1. `RuntimeOpenRequest` / `RuntimeOpenResult`
2. `GetScopeMetadataRequest` / `GetScopeMetadataResult`
3. `ListParametersRequest` / `ListParametersResult`
4. `GetParameterRequest` / `GetParameterResult`
5. `SetParameterRequest` / `SetParameterResult`

Required response fields remain:
1. `schema_version`
2. `status`
3. `model_hash`
4. `resolve_hash`
5. `error_count`
6. `warning_count`
7. `diagnostics`

## 6. Diagnostic Families (Frozen)
Runtime domain diagnostics (from runtime API):
1. `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION`
2. `E_RUNTIME_OPEN_INVALID`
3. `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` (ADR-0030 D2: `runtime-open` fails
   closed when the snapshot's `ccm_ref` does not resolve to a usable `.ccm`
   solver model — empty reference, unloadable artifact, or symbol-less stub)
4. `E_RUNTIME_HASH_MISMATCH`
5. `E_RUNTIME_UNKNOWN_SCOPE`
6. `E_RUNTIME_UNKNOWN_PATH`
7. `E_RUNTIME_TYPE_MISMATCH`
8. `E_RUNTIME_LIMIT_VIOLATION`
9. `E_RUNTIME_LIFECYCLE_IMMUTABLE`
10. `E_RUNTIME_ARTIFACT_UNKNOWN`

Selection-family diagnostics surfaced by `set-parameter` (the solver
option-validity pre-check, ADR-0017 §5 and ADR-0030 D4):
1. `E_SELECTION_CONFLICT`, `E_SELECTION_INVALID_OPTION`,
   `E_SELECTION_UNKNOWN_FACET`, `E_SELECTION_UNSATISFIABLE` (typed constraint
   violations)
2. `E_SELECTION_ENGINE_DIVERGENCE` (ADR-0030 D4: an internal solver fault on a
   modeled write fails closed instead of being silently skipped)

Runtime CLI transport diagnostics:
1. `E_RUNTIME_CLI_ARGS_INVALID`
2. `E_RUNTIME_CLI_REQUEST_IO`
3. `E_RUNTIME_CLI_REQUEST_TOO_LARGE`
4. `E_RUNTIME_CLI_REQUEST_INVALID`
5. `E_RUNTIME_CLI_RESPONSE_IO`

## 7. Security Invariants (Mandatory)
1. Fail-closed parser behavior for malformed envelopes.
2. Stable diagnostic code-family behavior across repeated failing requests.
3. Non-leaky stderr policy:
   - no stack traces
   - no request-payload secret echo
4. Explicit request/response file I/O classification (`not_found`, `permission_denied`, etc.).
5. Deterministic replay guarantees for read-path commands.
6. Write-path enforcement parity with runtime API policy checks:
   - lifecycle mutability
   - type compatibility
   - limits
   - artifact existence

## 8. Hash Lineage Continuity
For runtime operations derived from interpreter resolve outputs:
1. `runtime_open_result.model_hash == resolve_result.model_hash`
2. `runtime_open_result.resolve_hash == resolve_result.resolve_hash`
3. runtime read/write responses preserve the same `model_hash` + `resolve_hash`
4. runtime write/readback does not alter hash lineage identifiers

## 9. Verification Evidence
Implementation evidence is validated by runtime RUN matrix tests:
1. `runtime/src/main.rs` test cases `run_001` .. `run_020`
2. `//runtime:runtime_cli_test`
3. `//runtime:runtime_scenario_smoke_test`
4. `//runtime:runtime_scenario_medium_test`
5. `//runtime:runtime_system_e2e_gate_test`
