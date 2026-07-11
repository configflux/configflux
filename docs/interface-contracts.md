# ConfigFlux Interface Contracts (V1 RC Freeze)

This document freezes the implemented v1 product and loader contracts.
It is verified against code and tests in:
- `compiler/src/product_api.rs`
- `compiler/src/loader_api/`
- `compiler/src/runtime_api/`
- `compiler/src/ir.rs`

Verification date: 2026-03-14.

## 1) Scope

This contract freeze covers:
- compiler product API envelopes and diagnostics,
- compiler -> loader CMP manifest handoff,
- loader selection/resolve/export/BOM API envelopes and diagnostics,
- runtime reference API envelopes and diagnostics,
- deterministic hash and canonicalization behavior used by v1 tests.

Out of scope:
- transport protocol details,
- distributed runtime session orchestration,
- remote runtime sync/promotion policy.

### 1.1) Product Ownership Matrix (Interpreter Program)

This matrix is frozen for interpreter productization planning:

- Configuration Compiler owns:
  - source ingestion/link/verify
  - CMP emission and compiler diagnostics (`E_COMPILE_*`, graph checks)
- Interpreter (post-compile answer engine) owns:
  - CMP open + constrained selection + resolve + export + software BOM command surfaces
  - deterministic command I/O behavior and stable diagnostic family mapping
  - fail-closed input validation at command boundary
- Target Runtime Daemon owns:
  - target-side snapshot loading and runtime CRUD serving
  - runtime write-policy enforcement and target persistence lifecycle

Interpreter program out-of-scope (explicit):
- runtime daemon protocol freeze
- runtime persistence implementation details
- runtime sync/promotion policy

## 2) Shared Contract Rules

- Product/loader API `schema_version` is frozen at `3` (`1 → 2` ADR-0038, `2 → 3` ADR-0047).
- Operation status values are `ok` and `error`.
- All result envelopes include:
  - `schema_version`
  - `status`
  - `error_count`
  - `warning_count`
  - `diagnostics`
- Additive-optional compatibility rule: a new response field that is
  optional and omitted from the wire form when unset
  (serde `skip_serializing_if`) is a backward-compatible addition — it does
  NOT bump `schema_version` and does NOT enter any identity hash preimage.
  Consumers must ignore unknown optional fields. This is the mechanism behind
  `budget_report`, `progress_summary`, and `tool_version` (ADR-0044 D1).
- `diagnostics_ref` is currently always `null`/omitted in v1.
- `diagnostics` shape is:
  - `schema_version: u32`
  - `diagnostics: list<diagnostic>`
  - `error_count: u32`
  - `warning_count: u32`
- `diagnostic` shape is:
  - `code: string`
  - `severity: error | warning | info`
  - `message: string`
  - `source_id?: string`
  - `entity_path?: string`
  - `hint?: string`

Deterministic identity hashes in v1:
- `model_hash`
- `selection_state_hash`
- `resolve_hash`
- `generator_hash`
- `bom_hash`

## 3) Application 1: Compiler Product API

### 3.1 `verify_model`

Request:
- `schema_version: u32`
- `source_manifest: list<{ source_id: string, inline_content: string }>`

Result (`verify_report`):
- `schema_version: u32`
- `model_hash: string`
- `status: ok | error`
- `error_count: u32`
- `warning_count: u32`
- `checks: list<{ check_id, status, summary, diagnostic_codes }>`
- `diagnostics_ref?: string`
- `diagnostics: diagnostics_report`

Current v1 behavior:
- `check_id` is `graph_integrity`.
- Success returns one `pass` check.
- Failures return one `fail` check with stable diagnostic code(s).
- `verify_report.model_hash` is derived from the request `source_manifest` payload hash.

### 3.2 `compile_model`

Request:
- `schema_version: u32`
- `source_manifest: list<{ source_id, inline_content }>`
- `output_dir?: string`
- `stamp_time?: bool` (additive optional; ADR-0044 D1) — when `true`, the
  emitted `provenance.json` sidecars carry a wall-clock `stamped_at`. Default
  `false` keeps the compile byte-stable (same inputs → same bytes, sidecar
  included). Absent from the wire form when `false` (`skip_serializing_if`).

Result (`compile_result`):
- `schema_version: u32`
- `status: ok | error`
- `model_hash: string`
- `compiled_model_package_ref?: string` (points to emitted `cmp.manifest.json`)
- `diagnostics_ref?: string`
- `stats: { source_count, chunk_count, definition_count, component_count, artifact_count }`
- `verify_report: verify_report`
- `tool_version?: string` (additive optional; ADR-0044 D1) — the producing
  workspace version (from `/VERSION`). Carried on the side-channel like
  `budget_report`/`progress_summary`; never enters a hashed artifact byte.

Current v1 behavior:
- when `output_dir` is provided and emission succeeds, `compile_result.model_hash` is set to emitted index `config_hash` (CMP model identity).
- this can differ from `verify_report.model_hash` because verify hash is source-manifest-derived.
- when `output_dir` is provided and emission succeeds, a deterministic,
  NON-hashed `provenance.json` sidecar is written next to each file-writing
  artifact set — the CMP directory (`<out>/provenance.json`) and its sibling
  CCM directory (`<out>/ccm/provenance.json`). Each records `tool`,
  `tool_version`, the relevant `schema_versions`, and the SHA-256 content
  hashes of the artifacts it accompanies. The sidecar is outside every hash
  preimage and byte-stable across runs unless `stamp_time` is set (ADR-0044
  D1).

### 3.3 `inspect_model`

Request:
- `schema_version: u32`
- `source_manifest: list<{ source_id, inline_content }>`
- `query` (`query_type` tagged union):
  - `summary`
  - `component { component_id }`
  - `definition { definition_id }`
  - `artifact { artifact_id }`
  - `parameter { component_id, param_key }`
  - `scoped_stats { scope }`

Result (`inspection_result`):
- `schema_version: u32`
- `status: ok | error`
- `model_hash: string`
- `query`
- `summary: { source_count, definition_count, component_count, artifact_count, definition_ids, component_ids, artifact_ids }`
- `item?: inspection_item`
- `error_count: u32`
- `warning_count: u32`
- `diagnostics_ref?: string`
- `diagnostics: diagnostics_report`

`inspection_item` variants:
- `component { component_id, component_type, condition, depends_on, param_count, param_keys }`
- `definition { definition_id, param_type, inherits, has_value, override_count }`
- `artifact { artifact_id, name, version?, hash?, source?, target? }`
- `parameter { component_id, param_key, inherits?, type?, value?, unit?, lifecycle?, safety?, access?, req_id?, doc?, override_count, override_conditions, candidate_artifact_ids }`
- `scoped_stats { scope, scope_roots, component_count, parameter_count, artifact_count, component_ids, artifact_ids }`

### 3.4 Compiler Diagnostic Codes (Frozen)

- `E_UNKNOWN_COMPONENT_DEP`
- `E_COMPONENT_DEP_CYCLE`
- `E_COMPONENT_DEP_DIAMOND` — retired (ADR-0048), reserved, never reused
- `E_COMPILE_INPUT_INVALID`
- `E_COMPILE_EMIT_FAILED`
- `E_UNSUPPORTED_SCHEMA_VERSION`
- `E_INSPECT_UNKNOWN_COMPONENT`
- `E_INSPECT_UNKNOWN_DEFINITION`
- `E_INSPECT_UNKNOWN_ARTIFACT`
- `E_INSPECT_UNKNOWN_PARAMETER`
- `E_INSPECT_UNKNOWN_SCOPE`
- `E_INSPECT_QUERY_INVALID`

## 4) Compiler -> Loader Handoff (CMP)

Emitted CMP contents:
- `cmp.manifest.json`
- `index.cfir.json`
- `chunk-<hash>.cfir` files

`cmp.manifest.json` shape (`CmpManifest`):
- `schema_version: 1`
- `model_hash: string`
- `ir_format_version: 1`
- `index_ref: string` (default `index.cfir.json`)
- `chunk_set_ref: string` (default `.`)
- `config_hash: string`
- `hash_algo: "sha256"`
- `canonicalization_version: 1`
- `created_at: "1970-01-01T00:00:00Z"` (deterministic in v1)
- `stats?: { source_count, chunk_count, definition_count, component_count, artifact_count }`

Loader `open_model` validates:
- manifest parse and schema,
- manifest/index version consistency,
- index `config_hash` recomputation,
- manifest hash consistency (`model_hash`, `config_hash`, index hash),
- chunk integrity and presence,
- manifest stats consistency (when present).

## 5) Application 2: Loader API

### 5.1 `open_model`

Request:
- `schema_version: u32`
- `cmp_manifest_ref: string`

Result:
- `schema_version: u32`
- `status: ok | error`
- `model_hash?: string`
- `model_handle?: { model_hash, cmp_manifest_ref, index_ref, chunk_set_ref }`
- `error_count: u32`
- `warning_count: u32`
- `diagnostics_ref?: string`
- `diagnostics: diagnostics_report`

Open-model diagnostic codes (frozen):
- `E_LOADER_UNSUPPORTED_SCHEMA_VERSION`
- `E_LOADER_MANIFEST_INVALID`
- `E_LOADER_MANIFEST_INCONSISTENT`
- `E_LOADER_INDEX_INVALID`

### 5.2 Selection State and Guided Selection

`initialize_selection_state` request:
- `schema_version`
- `model_handle`
- `scope`
- `context_tags: map<string, string>`

`initialize_selection_state` result:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `selection_state?`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`

Generated `selection_state`:
- `choices` is always an empty map
- `selection_state_hash` is the canonical hash for the emitted tuple

`selection_state` shape:
- `schema_version: u32`
- `model_hash: string`
- `scope: string`
- `context_tags: map<string, string>`
- `choices: map<string, string>`
- `selection_state_hash: string`

`selection_state_hash`:
- sha256 over canonical JSON tuple:
  - `schema_version`
  - `model_hash`
  - `scope`
  - `context_tags` (sorted map)
  - `choices` (sorted map)

`get_selection_options` request:
- `schema_version`
- `model_handle`
- `scope`
- `selection_state`
- `facet`
- `include_pruned_reasons: bool`

`get_selection_options` result:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `facet`
- `valid_options: list<string>` (deterministic order)
- `pruned_options?: list<{ option, reason }>`
- `selection_state_hash`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`

`apply_selection` request:
- `schema_version`
- `model_handle`
- `scope`
- `selection_state`
- `selection_delta: { facet, option }`

`apply_selection` result:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `selection_state?`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`

`explain_rejection` request:
- `schema_version`
- `model_handle`
- `scope`
- `selection_state`
- `rejected_option: { facet, option }`

`explain_rejection` result:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `facet`
- `option`
- `rejection: { code, message, blocking_choices, hint?, unsat_core? }`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`

`rejection.unsat_core` (labeled minimal unsatisfiable subset, ADR-0031 D3):
```json
"unsat_core": {
  "rejected": { "facet": "<facet>", "option": "<option>" },
  "conflicting_constraints": [
    {
      "kind": "selection" | "model_rule",
      "facets": [ { "facet": "<facet>", "option": "<option>" } ],
      "summary": "<human-readable one-line constraint description>"
    }
  ],
  "minimal": true,
  "note": "one minimal explanation; other minimal cores may exist"
}
```
- `rejected` — the `(facet, option)` whose application is unsatisfiable, echoed
  for self-containment.
- `conflicting_constraints` — the labeled MUS: the minimal set of constraints
  that, together with `rejected`, are unsatisfiable. Each `facets` entry names a
  facet/option by its labeled `{facet}.{option}` name; this field MUST NOT
  contain raw BDD variable indices. `kind` is `selection` for a conflicting
  prior choice already in `selection_state`, or `model_rule` for a
  `requires`/`excludes`-style constraint baked into the `.ccm`. `summary` is
  advisory human-gloss text, not a parsed field.
- `minimal` — always `true` in this release (deletion-based extraction yields a
  minimal subset); reserved so a future fast path can set it `false` without a
  schema break.
- `note` — fixed advisory string acknowledging non-uniqueness. MUS extraction
  returns *a* minimal explanation, not *the* canonical one: two runs may yield
  different (equally minimal) cores when several exist. Callers needing stable
  output across runs key on the *set* of `conflicting_constraints`, understanding
  it is one witness among possibly several.

Presence rule: `unsat_core` is present exactly when `code` is
`E_SELECTION_CONFLICT` or `E_SELECTION_UNSATISFIABLE` — the genuine
"your selection contradicts the model" rejections that MUS extraction explains.
It is absent (the field is omitted) otherwise: for the division-of-labor
rejections owned without a solver core (`E_SELECTION_UNKNOWN_FACET`,
`E_SELECTION_INVALID_OPTION`, `E_SELECTION_STATE_INVALID`), and for fail-closed
command errors (`E_SELECTION_SOLVER_MODEL_UNAVAILABLE`,
`E_SELECTION_ENGINE_DIVERGENCE`), which are not rejections with a core.

Interpreter `explain` enrichment: the interpreter `explain` command returns this
same `explain_rejection` result, now carrying the `unsat_core` field above on
solver-decided constraint conflicts. The command shape, transport
(`--request-file`/`--response-file`, else stdin/stdout), and exit codes are
unchanged; the labeled core is added to the existing result payload.

Selection diagnostic codes (frozen):
- `E_SELECTION_STATE_INVALID`
- `E_SELECTION_UNKNOWN_FACET`
- `E_SELECTION_INVALID_OPTION`
- `E_SELECTION_CONFLICT`
- `E_SELECTION_UNSATISFIABLE`
- `E_SELECTION_SOLVER_MODEL_UNAVAILABLE` — ADR-0030 D1: `options`/`select` fail
  closed when no usable `.ccm` solver model is reachable (empty reference,
  unloadable artifact, or symbol-less stub). A usable `.ccm` is a hard
  precondition for the selection path; there is no silent fallback to the
  legacy compiler decision.
- `E_SELECTION_ENGINE_DIVERGENCE` — ADR-0030 D3/D4, internal-fault family:
  emitted when the solver rejects a `select` the legacy engine accepts (engine
  divergence — a correctness incident, not a tie to break), or when a
  solver-owned `options`/`select`/`set-parameter` query faults internally. Both
  fail closed rather than degrade to legacy.

### 5.3 Resolution

`resolve_from_selection` request:
- `schema_version`
- `model_handle`
- `scope`
- `selection_state`

`resolve_result`:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `selection_state_hash`
- `resolve_hash?`
- `resolved_output?: map<string, resolved_config>`
- `context_tags: map<string, string>`
- `choices: map<string, string>`
- `resolved_component_dependencies: map<string, map<string, list<string>>>`
- `resolved_artifacts: map<string, artifact_core>`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`

`resolve_hash` canonicalization:
- sha256 over canonical JSON payload:
  - `schema_version` (= 3)
  - `model_hash`
  - `scope`
  - canonical `selection_state` tuple
  - canonicalized `resolved_output` (object keys sorted lexicographically at every depth)

Resolution diagnostic codes (frozen):
- `E_RESOLVE_SCOPE_INVALID`
- `E_RESOLVE_MODEL_INVALID`
- `E_RESOLVE_CONTEXT_UNSATISFIED`
- `E_RESOLVE_FAILED`
- `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE` — ADR-0030 D1/D4: `resolve` fails closed
  when no usable `.ccm` solver model is reachable to gate satisfiability
  (absence, or a sat-gate fault). The solver sat-gate is no longer
  advisory-on-absence — the solver model is part of every resolve decision's
  lineage even though the compiler resolver would independently reject an
  unsatisfiable selection.
- plus selection-state validation code: `E_SELECTION_STATE_INVALID`

### 5.4 Early-Binding Export

`export_resolved` request:
- `schema_version`
- `resolve_result`
- `profile: string` (`cpp_early_binding_v1` in v1)

`export_resolved` result:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `resolve_hash?`
- `generated_artifacts?: { profile, generator_hash, files }`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`
- `tool_version?` (additive optional; ADR-0044 D1) — the producing
  workspace version. Envelope-only identity: `export_resolved` writes no
  output directory, so it carries no `provenance.json` sidecar. Absent from
  the wire form when unset (`skip_serializing_if`); adding it does NOT bump
  `schema_version` and it is outside every hash preimage (`resolve_hash` is
  computed over the selection/resolved output, not this envelope).

`generated_file` shape:
- `path: string`
- `contents: string`
- `content_hash: string`

Frozen output paths (`profile = cpp_early_binding_v1`):
- `generated/config.hpp`
- `generated/config_artifact_manifest.json`
- `generated/config_build_flags.cmake`

Artifact manifest file shape:
- `schema_version`
- `profile`
- `model_hash`
- `resolve_hash`
- `artifacts: list<{ artifact_id, bound_paths }>`

Early-binding behavior:
- only `lifecycle = construction` parameters are emitted into C++/CMake outputs,
- artifact manifest is sourced from selected construction artifact bindings.

Export diagnostic codes (frozen):
- `E_EXPORT_PROFILE_INVALID`
- `E_EXPORT_RESOLVE_INVALID`
- `E_EXPORT_ARTIFACT_INVALID`
- `E_EXPORT_SYMBOL_INVALID`
- `E_EXPORT_FAILED`

`generator_hash` canonicalization:
- sha256 over canonical tuple:
  - `schema_version`
  - `profile`
  - `model_hash`
  - `scope`
  - `resolve_hash`
  - `files: list<{ path, content_hash }>`

### 5.5 Software BOM Export

`export_software_bom` request:
- `schema_version`
- `resolve_result`
- `profile: full_audit | value_redacted`

`export_software_bom` result:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `resolve_hash?`
- `bom_hash?`
- `software_bom?` (`SoftwareBomV1`)
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`
- `tool_version?` (additive optional; ADR-0044 D1) — the producing
  workspace version, carried on the RESULT envelope only. The hashed
  `SoftwareBomV1.generator` (frozen at `0.1.0`) and its zero-default
  `generated_at` are unchanged, so `bom_hash` is unaffected; this field is
  absent from the wire form when unset and does NOT bump `schema_version`.

Software BOM payload contract is frozen in `docs/software-bom-schema.md`.

Software BOM diagnostic codes (frozen):
- `E_SBOM_PROFILE_INVALID`
- `E_SBOM_RESOLVE_INVALID`
- `E_SBOM_PATH_INVALID`
- `E_SBOM_ARTIFACT_INVALID`
- `E_SBOM_BINDING_INVALID`
- `E_SBOM_STATS_INVALID`
- `E_SBOM_HASH_INVALID`
- `E_SBOM_FAILED`

### 5.6 Runtime Delivery Slice (Loop 10)

`runtime_open` request (`runtime_open_request`):
- `schema_version`
- `model_hash`
- `resolve_hash`
- `scope`
- `resolved_output` (`resolve_result.resolved_output`)
- `resolved_component_dependencies`
- `resolved_artifacts`
- `context_tags` (optional; used for strict hash-mismatch validation)
- `choices` (optional; used for strict hash-mismatch validation)

`runtime_open` result:
- `schema_version`
- `status`
- `model_hash`
- `resolve_hash`
- `scope`
- `runtime_snapshot?`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`

`runtime_snapshot` shape:
- `schema_version`
- `model_hash`
- `resolve_hash`
- `scope`
- `context_tags`
- `choices`
- `resolved_output`
- `resolved_component_dependencies`
- `resolved_artifacts`

Read API in Loop 10:
- `get_scope_metadata(scope_root)`:
  - result metadata: `{ component_count, parameter_count, artifact_count }`
- `get_parameter(path)`:
  - `path` format: `component.<component_id>.param.<param_key>`
  - payload includes resolved metadata/value plus artifact binding metadata for `type=artifact`.
- `list_parameters(scope_root)`:
  - returns `parameter_paths` sorted lexicographically.

Write validation slice used by Loop 10 mutation tests:
- `set_parameter(path, value)`:
  - writable only when `lifecycle=runtime`.
  - enforces type compatibility.
  - enforces numeric/string limits when configured.
  - enforces artifact-ID existence in `resolved_artifacts` for `type=artifact`.

Runtime diagnostic codes (frozen):
- `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION`
- `E_RUNTIME_OPEN_INVALID`
- `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` — ADR-0030 D2: `runtime-open` fails
  closed when the snapshot's `ccm_ref` does not resolve to a usable `.ccm`
  solver model (empty reference, unloadable artifact, or symbol-less stub). A
  usable `.ccm` is a hard precondition enforced at open time; this is new
  behavior (pre-ADR-0030, `runtime-open` did not touch the `.ccm`). The
  precondition applies uniformly to **both** open entrypoints — the runtime CLI
  `runtime-open` handler and the C ABI `configflux_runtime_session_open`
  export — via a single shared enforcement point in the runtime crate
  (ADR-0030 Amendment 1). On the C ABI surface the rejection is carried in the
  open response envelope (`status=error` + this code) with no session handle
  returned; the boundary status remains `Ok`.
- `E_RUNTIME_HASH_MISMATCH`
- `E_RUNTIME_UNKNOWN_SCOPE`
- `E_RUNTIME_UNKNOWN_PATH`
- `E_RUNTIME_TYPE_MISMATCH`
- `E_RUNTIME_LIMIT_VIOLATION`
- `E_RUNTIME_LIFECYCLE_IMMUTABLE`
- `E_RUNTIME_ARTIFACT_UNKNOWN`
- `E_SELECTION_ENGINE_DIVERGENCE` — ADR-0030 D4: a `set-parameter` write to a
  modeled facet whose solver query faults internally fails closed (the retired
  ADR-0017 §5 rule silently skipped the check). Shares the selection surface's
  internal-fault code.

CCM precondition (ADR-0030 D2): a usable `.ccm` solver model is required for
`runtime-open`. After a valid open, every snapshot carries a loadable `.ccm`, so
`set-parameter` no longer skips the solver constraint check on availability — it
only skips the permanent division-of-labor cases (non-string value, non-facet
path, unconstrained facet).

### 5.7 Runtime CLI Binary Contract (Loop 11)

Binary command surface (`configflux-runtime`):
- `runtime-open`
- `get-scope-metadata`
- `list-parameters`
- `get-parameter`
- `set-parameter`
- `explain-rejection`

`explain-rejection` (read-side, solver-decided query; parallel to
`set-parameter`): explains why a `set-parameter` selection would be rejected,
against the same `.ccm` the open snapshot already validates against. It is a pure
query — it does not mutate the session. The request carries `schema_version`, the
runtime snapshot, and the candidate `{parameter, value}` pair (`path` / `value`);
the result echoes `path` / `value` and returns the same
`rejection: { code, message, blocking_choices, hint?, unsat_core? }` payload as
the loader `explain_rejection` result (§5.2), including the labeled `unsat_core`
on solver-decided constraint conflicts. The runtime's `{parameter, value}`
vocabulary maps to the model's `{facet, option}`: `path` names the candidate
facet, a string `value` names the candidate option. A rejection explanation is a
success (`status = ok`, exit `0`); exit `2` is reserved for "could not compute an
explanation at all" (model unavailable or solver fault, fail-closed per
ADR-0030/ADR-0031 D4). An unknown `{parameter, value}` pair is a division-of-labor
case (`E_SELECTION_UNKNOWN_FACET` / `E_SELECTION_INVALID_OPTION`) with no core.

Transport policy:
- stdin/stdout mode by default.
- file mode via `--request-file` and `--response-file`.
- bounded request size (`8 MiB`) with fail-closed behavior.
- exit-code mapping:
  - `0`: command result `status = ok`
  - `2`: command result `status = error`
  - `1`: transport/CLI misuse or I/O failure

Runtime CLI transport diagnostics (frozen):
- `E_RUNTIME_CLI_ARGS_INVALID`
- `E_RUNTIME_CLI_REQUEST_IO`
- `E_RUNTIME_CLI_REQUEST_TOO_LARGE`
- `E_RUNTIME_CLI_REQUEST_INVALID`
- `E_RUNTIME_CLI_RESPONSE_IO`

Detailed operator contract and examples are frozen in:
- `docs/runtime-cli-contract.md`
- `docs/runtime-run-matrix.md`
- `docs/runtime-v2-operations-runbook.md`

## 6) Determinism Guarantees (v1)

For identical inputs:
- `model_hash` is stable.
- `selection_state_hash` is stable.
- `resolve_hash` is stable.
- early-binding file contents and `generator_hash` are stable.
- software BOM payload bytes and `bom_hash` are stable.

Equivalent map insertion order does not change:
- `selection_state_hash`
- `resolve_hash`
- `generator_hash`
- `bom_hash`
- runtime read API response bytes for identical snapshot + request.

## 7) Known Limits / Non-Goals (V1)

- Runtime API is currently an in-process reference slice (no protocol/server freeze).
- No distributed or persisted selection-session API in v1 (stateless APIs are canonical).
- Loader APIs are library APIs; CLI wrappers for selection/resolve/export are not part of v1 freeze.
- Full SPDX/CycloneDX mappings are deferred; v1 exports native `SoftwareBomV1`.
