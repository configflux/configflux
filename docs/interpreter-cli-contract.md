# Interpreter CLI Product Contract v1 (Frozen)

Status: frozen and implemented
Date: 2026-02-14
Current status source: [CHANGELOG.md](../CHANGELOG.md)

## 1. Scope
This contract defines the interpreter command surface between compiler CMP outputs
and downstream resolve/export consumers.

This document describes the implemented interpreter v1 behavior, including
request/response envelope semantics, deterministic I/O, and security
invariants.

## 2. Commands
Interpreter v1 commands:
1. `open`
2. `init-selection-state`
3. `options`
4. `select`
5. `explain`
6. `resolve`
7. `export-resolved`
8. `export-software-bom`

## 3. I/O Modes
Default mode:
- request JSON via `stdin`
- response JSON via `stdout`

Optional file mode:
- request JSON via `--request-file <path>`
- response JSON via `--response-file <path>`

Determinism rules:
- output must be stable for identical inputs
- no nondeterministic metadata in envelopes
- object/list ordering follows existing frozen loader/runtime contract behavior

### 3.1 Operator/Developer Command Examples (`configflux-interpreter`)
Examples below use `schema_version = 1` and JSON envelopes from
`docs/interface-contracts.md`.
Use `docs/canonical-worked-example.md` for the canonical end-to-end operator
flow, including the shipped helper for initial `selection_state` generation.

1. `open` (stdin/stdout mode):
```bash
echo '{
  "schema_version": 1,
  "cmp_manifest_ref": "out/cmp/cmp.manifest.json"
}' | configflux-interpreter open > out/open.result.json
```

2. `init-selection-state`:
```bash
configflux-interpreter init-selection-state \
  --request-file requests/init-selection-state.request.json \
  --response-file out/init-selection-state.result.json
```
`requests/init-selection-state.request.json`:
```json
{
  "schema_version": 1,
  "model_handle": {
    "model_hash": "<model_hash>",
    "cmp_manifest_ref": "out/cmp/cmp.manifest.json",
    "index_ref": "out/cmp/index.cfir.json",
    "chunk_set_ref": "out/cmp"
  },
  "scope": "component:thermal_control",
  "context_tags": {
    "region": "us"
  }
}
```

3. `options` (request/response file mode):
```bash
configflux-interpreter options \
  --request-file requests/options.request.json \
  --response-file out/options.result.json
```
`requests/options.request.json`:
```json
{
  "schema_version": 1,
  "model_handle": {
    "model_hash": "<model_hash>",
    "cmp_manifest_ref": "out/cmp/cmp.manifest.json",
    "index_ref": "out/cmp/index.cfir.json",
    "chunk_set_ref": "out/cmp"
  },
  "scope": "component:thermal_control",
  "selection_state": {
    "schema_version": 1,
    "model_hash": "<model_hash>",
    "scope": "component:thermal_control",
    "context_tags": {
      "region": "us"
    },
    "choices": {},
    "selection_state_hash": "<selection_state_hash>"
  },
  "facet": "cooling_brand",
  "include_pruned_reasons": true
}
```

4. `select`:
```bash
configflux-interpreter select \
  --request-file requests/select.request.json \
  --response-file out/select.result.json
```
`requests/select.request.json`:
```json
{
  "schema_version": 1,
  "model_handle": {
    "model_hash": "<model_hash>",
    "cmp_manifest_ref": "out/cmp/cmp.manifest.json",
    "index_ref": "out/cmp/index.cfir.json",
    "chunk_set_ref": "out/cmp"
  },
  "scope": "component:thermal_control",
  "selection_state": {
    "schema_version": 1,
    "model_hash": "<model_hash>",
    "scope": "component:thermal_control",
    "context_tags": {
      "region": "us"
    },
    "choices": {},
    "selection_state_hash": "<selection_state_hash>"
  },
  "selection_delta": {
    "facet": "cooling_brand",
    "option": "hydra"
  }
}
```

5. `explain`:
```bash
configflux-interpreter explain \
  --request-file requests/explain.request.json \
  --response-file out/explain.result.json
```
`requests/explain.request.json`:
```json
{
  "schema_version": 1,
  "model_handle": {
    "model_hash": "<model_hash>",
    "cmp_manifest_ref": "out/cmp/cmp.manifest.json",
    "index_ref": "out/cmp/index.cfir.json",
    "chunk_set_ref": "out/cmp"
  },
  "scope": "component:thermal_control",
  "selection_state": {
    "schema_version": 1,
    "model_hash": "<model_hash>",
    "scope": "component:thermal_control",
    "context_tags": {
      "region": "us"
    },
    "choices": {
      "cooling_brand": "hydra"
    },
    "selection_state_hash": "<selection_state_hash>"
  },
  "rejected_option": {
    "facet": "cooling_model",
    "option": "z900"
  }
}
```

6. `resolve`:
```bash
configflux-interpreter resolve \
  --request-file requests/resolve.request.json \
  --response-file out/resolve.result.json
```
`requests/resolve.request.json`:
```json
{
  "schema_version": 1,
  "model_handle": {
    "model_hash": "<model_hash>",
    "cmp_manifest_ref": "out/cmp/cmp.manifest.json",
    "index_ref": "out/cmp/index.cfir.json",
    "chunk_set_ref": "out/cmp"
  },
  "scope": "component:thermal_control",
  "selection_state": {
    "schema_version": 1,
    "model_hash": "<model_hash>",
    "scope": "component:thermal_control",
    "context_tags": {
      "region": "us"
    },
    "choices": {
      "cooling_brand": "hydra",
      "cooling_model": "x200",
      "pump_type": "dual"
    },
    "selection_state_hash": "<selection_state_hash>"
  }
}
```

7. `export-resolved`:
```bash
jq -n --slurpfile rr out/resolve.result.json '{
  schema_version: 1,
  resolve_result: $rr[0],
  profile: "cpp_early_binding_v1"
}' > requests/export-resolved.request.json

configflux-interpreter export-resolved \
  --request-file requests/export-resolved.request.json \
  --response-file out/export-resolved.result.json
```

8. `export-software-bom`:
```bash
jq -n --slurpfile rr out/resolve.result.json '{
  schema_version: 1,
  resolve_result: $rr[0],
  profile: "full_audit"
}' > requests/export-software-bom.request.json

configflux-interpreter export-software-bom \
  --request-file requests/export-software-bom.request.json \
  --response-file out/export-software-bom.result.json
```

Expected identity continuity across command results:
- `open`: emits `model_hash`.
- `init-selection-state`: emits canonical `selection_state` with `selection_state_hash`.
- `options`/`select`: preserve or update `selection_state_hash`.
- `resolve`: emits `resolve_hash` bound to `model_hash` + `selection_state_hash`.
- `export-resolved` / `export-software-bom`: preserve `model_hash` + `resolve_hash`.

## 4. Exit Codes
1. `0`: command result `status = ok`
2. `2`: command result `status = error`
3. `1`: CLI transport/parsing misuse (invalid JSON, invalid args, I/O failure)

## 5. Request/Response Envelope Policy
Mapping policy:
- Command envelopes should map directly to existing library request/result models in
  `compiler` loader APIs.

Required shared fields in results:
- `schema_version`
- `status`
- `error_count`
- `warning_count`
- `diagnostics`

Additive-optional fields: results MAY carry backward-compatible optional
fields that are omitted from the wire form when unset (serde
`skip_serializing_if`). Such additions do NOT bump `schema_version` and never
enter an identity hash preimage; consumers must ignore unknown optional
fields. The `export-resolved` and `export-software-bom` results carry
`tool_version?` (the producing workspace version) under this rule — these are
envelope-only builders (no output directory), so they carry identity on the
envelope rather than in a `provenance.json` sidecar (ADR-0044 D1). Because
`tool_version` is a build constant, §6.7 deterministic replay is unaffected.

Diagnostics shape is inherited from frozen product contracts in
`docs/interface-contracts.md`.

## 6. Security Invariants (Mandatory)
1. Fail-closed parsing:
   - malformed JSON and unsupported schema are rejected deterministically.
2. Envelope validation:
   - required fields must be present and type-valid.
3. Path handling:
   - request/response file path errors are explicit and non-ambiguous.
4. Bounded input policy:
   - oversized requests are rejected with deterministic error class.
5. Non-leaky diagnostics:
   - no stack traces, secrets, or sensitive host internals in contract-facing output.
6. Stable diagnostics:
   - selection/resolve/export/SBOM diagnostic code families remain stable.
7. Deterministic replay:
   - repeated identical requests produce byte-identical response JSON.

## 7. Security-Relevant Command Behavior Freeze
The following command families are security-sensitive and must preserve stable
validation/error semantics:
1. `open` -> `E_LOADER_*`
2. `init-selection-state` -> `E_LOADER_*`, `E_SELECTION_STATE_INVALID`
3. `options`, `select`, `explain` -> `E_SELECTION_*`
4. `resolve` -> `E_RESOLVE_*`
5. `export-resolved` -> `E_EXPORT_*`
6. `export-software-bom` -> `E_SBOM_*`

CCM precondition (ADR-0030, fail closed): `options`, `select`, and `resolve`
require a usable `.ccm` solver model (loadable artifact with a populated symbol
table). When none is reachable they fail closed —
`E_SELECTION_SOLVER_MODEL_UNAVAILABLE` for `options`/`select`,
`E_RESOLVE_SOLVER_MODEL_UNAVAILABLE` for `resolve` — rather than degrading to the
legacy compiler decision. A solver fault on a modeled query, or a solver/legacy
engine divergence on the `select` reject path, fails closed with
`E_SELECTION_ENGINE_DIVERGENCE`. The permanent division of labor (unconstrained
facets via `E_SELECTION_UNKNOWN_FACET`, schema/empty validation, envelope
composition) stays compiler-owned and is unchanged.

## 8. Integration Scenario Matrix
Interpreter implementation must satisfy this matrix:
1. `INT-001` happy-path E2E
2. `INT-002` corrupted CMP artifacts
3. `INT-003` malformed request envelopes
4. `INT-004` selection tampering
5. `INT-005` resolve integrity failures
6. `INT-006` export misuse
7. `INT-007` determinism replay
8. `INT-008` diagnostic leakage assertions
9. `INT-009` bounded-input hardening
10. `INT-010` traceability/hash lineage completeness

## 9. CRA Mapping Appendix (Engineering Readiness)
Baseline regulation: EU CRA (Regulation (EU) 2024/2847).

Anchors:
- Entered into force: 2024-12-10
- Main obligations apply: 2027-12-11
- Vulnerability/incident reporting obligations apply: 2026-09-11

Engineering control mapping:
1. Secure by default -> strict validation + fail-closed behavior.
2. Vulnerability handling support -> deterministic diagnostics and incident triage runbook.
3. Component traceability -> deterministic export + SBOM hash lineage.
4. Update/compliance evidence -> reproducible CI gates and archived test artifacts.

References:
- https://digital-strategy.ec.europa.eu/en/policies/cyber-resilience-act
- https://eur-lex.europa.eu/eli/reg/2024/2847/oj/eng
