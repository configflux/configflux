# ConfigFlux Software BOM Schema (V1 RC Freeze)

This document freezes the implemented `SoftwareBomV1` payload and validation rules.
It is verified against `compiler/src/loader_api/` on 2026-03-14.

## 1) Scope

In scope:
- v1 payload shape for software BOM export,
- deterministic canonicalization and `bom_hash` rules,
- validation checks enforced by exporter/validator,
- profile behavior for `full_audit` and `value_redacted`.

Out of scope:
- SPDX/CycloneDX export mappings,
- transport protocol details.

## 2) Canonical Payload Shape

Top-level object (`SoftwareBomV1`):
- `schema_version: u32` (`1` in v1)
- `bom_version: u32` (`1` in v1)
- `bom_hash: string`
- `hash_algo: string` (`"sha256"` in v1)
- `canonicalization_version: u32` (`1` in v1)
- `model_hash: string`
- `resolve_hash: string`
- `selection_state_hash?: string`
- `scope_root: string`
- `generated_at: string` (deterministic v1 constant: `"1970-01-01T00:00:00Z"`)
- `generator: { name: string, version: string }`
  - v1 constants:
    - `name = "configflux-sbom"`
    - `version = "0.1.0"`
- `context_tags: map<string, string>`
- `choices: map<string, string>`
- `components: list<ComponentBomEntry>`
- `parameters: list<ParameterBomEntry>`
- `artifacts: list<ArtifactBomEntry>`
- `stats: { component_count, parameter_count, artifact_count }`

## 3) Entry Shapes

### 3.1 ComponentBomEntry
- `component_id: string`
- `type: string`
- `dependency_ids: list<string>`

### 3.2 ParameterBomEntry
- `path: string` (`component.<component_id>.param.<param_key>`)
- `component_id: string`
- `param_key: string`
- `type: string`
- `value: scalar` (`int | float | bool | string`)
- `unit?: string`
- `safety: string`
- `lifecycle: string`
- `binding_phase: early | late | runtime`
- `access: string`
- `req_id?: string`
- `doc?: string`
- `limits?: object`

### 3.3 ArtifactBomEntry
- `artifact_id: string`
- `bound_paths: list<string>`
- `name: string`
- `version?: string`
- `hash?: string`
- `source?: string`
- `target?: string`
- `doc?: string`

## 4) Deterministic Hashing

`bom_hash` is computed over canonical JSON bytes for the full payload with this rule:
- clone payload,
- set cloned `bom_hash = ""`,
- recursively sort all object keys lexicographically,
- preserve array order from canonical payload construction,
- serialize without insignificant whitespace,
- hash bytes with sha256.

V1 invariants:
- `hash_algo` must be `"sha256"`.
- `canonicalization_version` must be `1`.
- `bom_version` must be `1`.

## 5) Canonical Ordering Rules in Emitted BOM

The emitter builds sorted arrays to ensure deterministic payload order:
- `components` sorted by `component_id`.
- each `components[].dependency_ids` sorted and unique.
- `parameters` sorted by `path`.
- `artifacts` sorted by `artifact_id`.
- each `artifacts[].bound_paths` sorted and unique.

## 6) Validation Rules (Enforced)

Identity and version checks:
- `model_hash` must be non-empty.
- `resolve_hash` must be non-empty.
- `hash_algo`, `canonicalization_version`, `bom_version` must match v1 constants.

Path and binding checks:
- each parameter path must equal `component.<component_id>.param.<param_key>`.
- `binding_phase` must match lifecycle mapping:
  - `construction -> early`
  - `startup -> late`
  - `runtime -> runtime`

Dependency and artifact linkage checks:
- every dependency ID must reference a component present in `components`.
- `artifact`-typed parameters must contain non-empty string artifact IDs.
- every artifact parameter path must map to exactly one artifact bound path.
- every artifact bound path must reference an existing parameter path.
- artifact metadata entries must include non-empty `name`.

Stats and hash checks:
- `stats.*` must match actual list lengths.
- `bom_hash` must equal canonical sha256 hash.

## 7) Profiles

### 7.1 `full_audit`
- exports full parameter values.

### 7.2 `value_redacted`
- for each parameter:
  - if `binding_phase != early` and `type != "artifact"`, replace `value` with `"<redacted>"`.
  - otherwise preserve original value.
- structure, identity fields, paths, lifecycle, access, artifacts, and hashes are preserved.

## 8) Frozen Diagnostic Codes

Software BOM export/validation diagnostics use:
- `E_SBOM_PROFILE_INVALID`
- `E_SBOM_RESOLVE_INVALID`
- `E_SBOM_PATH_INVALID`
- `E_SBOM_ARTIFACT_INVALID`
- `E_SBOM_BINDING_INVALID`
- `E_SBOM_STATS_INVALID`
- `E_SBOM_HASH_INVALID`
- `E_SBOM_FAILED`
