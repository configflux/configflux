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
5. `set-parameter` -> `set_parameter(SetParameterRequest)`, followed by the
   solver constraint check over the session's total known assignment (fail
   closed on a solver fault, ADR-0030 D4)
6. `set-parameters-atomically` ->
   `set_parameters_atomically(SetParametersAtomicallyRequest)`, likewise
   followed by the solver constraint check — applied to the batch as a whole,
   so writes that are individually valid but jointly violating are rejected
   together
7. `commit-configuration` ->
   `commit_configuration(CommitConfigurationRequest)`, likewise followed by the
   solver constraint check

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

Selection-family diagnostics surfaced by `set-parameter`,
`set-parameters-atomically` and `commit-configuration` (the solver constraint
check, ADR-0017 §5 and its 2026-08-03 amendment, and ADR-0030 D4):
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

Upstream precondition (ADR-0054 §2/§6). A `resolve` whose total post-default
assignment violates a constraint the model declares is rejected with
`E_SELECTION_CONFLICT` and produces neither a `resolve_hash` nor a
`resolved_output`. There is therefore no snapshot for the runtime to open, and
no partial artifact to open by mistake: a configuration that broke a declared
policy cannot enter the runtime's lineage at all. This tightens what reaches
`runtime-open`; it changes no runtime command, envelope, exit code, or
diagnostic family.

## 9. Verification Evidence
Implementation evidence is validated by runtime RUN matrix tests:
1. `runtime/src/tests.rs` test cases `run_001` .. `run_050`
2. `//runtime:runtime_cli_test`
3. `//runtime:runtime_scenario_smoke_test`
4. `//runtime:runtime_scenario_medium_test`
5. `//runtime:runtime_system_e2e_gate_test`

## 10. Read Command Request Examples
Concrete request bodies for the read commands `get-scope-metadata`,
`list-parameters`, and `get-parameter`. Every read request threads the
`runtime_snapshot` object returned by `runtime-open` (see the open request in
`docs/service-integration-guide.md`) and adds a command-specific selector.
`schema_version` is the current product schema version (`4`). The snapshot is
abbreviated below as `{ "...": "from runtime-open" }`; pass the full object
through unchanged.

`get-scope-metadata` — component/parameter/artifact counts for one scope root:

```json
{
  "schema_version": 4,
  "runtime_snapshot": { "...": "from runtime-open" },
  "scope_root": "runtime_tuner"
}
```

`list-parameters` — the sorted parameter paths in one scope root:

```json
{
  "schema_version": 4,
  "runtime_snapshot": { "...": "from runtime-open" },
  "scope_root": "runtime_tuner"
}
```

`get-parameter` — one parameter's value and metadata. `path` is a
`component.<component_id>.param.<param_key>` path (one of the paths
`list-parameters` returns):

```json
{
  "schema_version": 4,
  "runtime_snapshot": { "...": "from runtime-open" },
  "path": "component.runtime_tuner.param.max_rpm"
}
```

A malformed request envelope fails closed with `E_RUNTIME_CLI_REQUEST_INVALID`
(exit `1`) and the diagnostic names the offending field — for example a request
that omits `scope_root` reports a missing required `scope_root` field. Field
*values* are never echoed (§7).
