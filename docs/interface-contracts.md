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

Section 8 registers the one envelope owned by the `cfx` presentation CLI rather
than by an application API. It is explicitly OUTSIDE this freeze — see §7 and §8
— and is recorded here so the repository has a single place that lists every
JSON shape and exit code the product emits.

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

- Product/loader API `schema_version` is frozen at `5` (`1 → 2` ADR-0038, `2 → 3` ADR-0047, `3 → 4` ADR-0054, `4 → 5` ADR-0057 §D7).
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

Deterministic but NOT an identity:
- `source_digest` — covers the request `source_manifest` contents and nothing
  else. It is reported by `inspect_model` (§3.3), which emits no package, so it
  never denotes a compiled model the way `model_hash` does (ADR-0056 §9).

## 3) Application 1: Compiler Product API

### 3.1 `verify_model`

Request:
- `schema_version: u32`
- `source_manifest: list<{ source_id: string, inline_content: string }>`

Result (`verify_report`):
- `schema_version: u32`
- `model_hash?: string` — present only when the model verified; equals the CMP
  model identity `compile` emits for the same sources (ADR-0056 Amendment 2).
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
- `verify_report.model_hash` is the CMP model identity: `verify` builds the
  model index in memory — writing nothing — and reports the `config_hash` that
  index carries, which is bit-identical to the `model_hash` `compile` emits for
  the same sources. When verification fails the field is omitted entirely; a
  report with no identity is a model that was never indexed. The
  source-manifest digest is `inspect_model`'s `source_digest` (§3.3) and
  `verify` does not repeat it.

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
- `verify_report.model_hash` carries that same identity on the success path, and is omitted on every error path — a failed compile emits no package and therefore has no model identity to report. `compile_result.model_hash` (the envelope's own field) keeps its provisional pre-emit value on those paths; the two fields are not interchangeable and only the nested one is absent when there is nothing to name.
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
- `source_digest: string` — a digest of the request `source_manifest` contents.
  Deliberately not named `model_hash`: `inspect_model` emits no package, so it
  never carries the CMP model identity that name denotes in §3.2.
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
- `parameter { component_id, param_key, inherits?, type?, value?, facet?, unit?, lifecycle?, safety?, access?, req_id?, doc?, override_count, override_conditions, candidate_artifact_ids }`
  - `facet?` — the facet this parameter is the declared handle for (ADR-0064).
    Mirrors `ResolvedParameter.facet`; **absent** for a parameter that declares
    no binding, so every existing inspect envelope is byte-unchanged. A bound
    parameter authors no `value`, so this is what accounts for the value it
    resolves to.
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
- `E_INGEST_DUPLICATE_FACET` — a facet declared by more than one chunk (ADR-0047 §2)
- `E_INGEST_DUPLICATE_CATALOGUE`
- `E_CATALOGUE_INVALID`
- `E_BINDING_INVALID`
- `E_REQUIRES_INVALID`
- `E_BINDING_NO_ACCEPTABLE_ENTRY`
- `E_FACET_VALUE_UNDECLARED` — an undeclared facet or a value outside a closed facet's declared domain (ADR-0047 §3)
- `E_OBJECT_UNIT_MISMATCH`
- `E_LINK_DUPLICATE_UNIT`
- `E_LINK_DUPLICATE_ID`
- `E_LINK_UNRESOLVED_IMPORT`
- `E_LINK_INTERFACE_MISMATCH`
- `E_LINK_OBJECT_CORRUPT`
- `E_LINK_LOCK_MISMATCH`
- `E_LINK_LOCK_UNLINKED`
- `E_LINK_LOCK_INVALID`

The eight `E_LINK_*` codes are faults *between* objects, raised by `link` from
the object headers alone. The three `E_LINK_LOCK_*` codes are §3.7's and are
raised only when `--lock` or `--write-lock` is given. They do not replace the codes above: a one-shot
`compile` holds the whole model, checks it before it links, and still reports
`E_UNKNOWN_COMPONENT_DEP` for a `depends_on` target nothing declares and
`E_REQUIRES_INVALID` for a requirement naming no declared binding. `link` can
be handed a subset of the objects, so for the same fault it has only the
headers to go on and reports `E_LINK_UNRESOLVED_IMPORT`, naming the unit and
the missing id. Neither form accepts a model the other rejects.

`docs/diagnostics.md` is the generated per-code registry, carrying a cause and a
remedy for every code the compiler, interpreter and runtime emit; the frozen
list above is its product-API subset.

### 3.5 Object Header Contract (v1)

`compile-object` writes an object directory holding `object.json`, one
`chunk-<hash>.cfir` per source chunk, and `provenance.json`. `object.json` is
the frozen v1 header (ADR-0058 §D2, amended §A1):

| Field | Type | Meaning |
| --- | --- | --- |
| `format_version` | `u32` | `1`. A header of another version is rejected, not read under this one |
| `unit` | `string` | the `package` value every chunk of the unit declares |
| `chunk_hashes` | `list<string>` | the unit's chunk hashes, ascending lowercase hex |
| `exports` | 7 sorted id lists | `definitions`, `components`, `artifacts`, `facets`, `bindings`, `catalogues`, `constraints` |
| `imports` | 5 sorted id lists | `components`, `definitions`, `facets`, `bindings`, `catalogues` -- referenced and not declared here |
| `facet_domains` | `map<facet_id, list<value>>` | declared value domains, in declared order |
| `open_facets` | `list<facet_id>` | of `facet_domains`, the facets declared `open: true` |
| `catalogue_entries` | `map<catalogue_id, list<entry_id>>` | entry rosters, id-ascending |
| `binding_links` | `map<binding_id, BindingLink>` | `catalogue`, `default`, `derive_source`, `derive_pairs`, `derive_source_count` |
| `requirements` | `list<RequirementLink>` | `component`, `slot`, `binding`, `accepts`, `condition`; component-then-slot ascending |
| `clauses` | `list<{id, condition}>` | authored constraints, chunk order then in-chunk order |
| `selectors` | `list<{id, condition}>` | inclusion selectors -- every component and override `condition`, keyed by the authored entity path that carries it, in canonical clause order |
| `interfaces` | `list<{unit, object_hash}>` | the interface objects this unit was compiled against, sorted |
| `object_hash` | `string` | SHA-256 of the header serialized WITHOUT this field |

`open_facets` sits beside `facet_domains`, and `selectors` beside `clauses`,
because the constraint model is a **link** product: the linker builds it from
headers, so a header must carry everything the model reads. A facet's `open`
flag decides whether the cardinality channel asserts at-least-one, and the
selectors are what introduce the `(facet, value)` symbols. Without them a
linker could not build the model without opening a chunk file, which is the one
thing an object header exists to avoid.

The **canonical clause order** (ADR-0058 §A2) is objects by unit name
ascending, then chunks by `chunk_hash` ascending, then, within a chunk,
definitions by id, components by id, and override order. `compile` and `link`
both build the constraint model in that order, so neither `--source` order nor
`--object` order can change a byte.

Guarantees:

- **No path.** No `source_id` and no filesystem path appears in the header. Paths
  survive only inside the chunk files, as provenance.
- **Path- and order-invariant identity.** `object_hash` is unchanged by the
  spelling or the order of the `--source` arguments.
- **Byte-stable.** The same unit compiled twice yields a byte-identical object
  directory, sidecar included, unless `--stamp-time` is passed.
- **Recomputable.** A reader recomputes `object_hash` from the header and
  compares; a mismatch means the header was edited after it was written.
- **Header-only reads.** Compiling against an interface reads only its
  `object.json`.

### 3.6 `link_model`

Request:
- `schema_version: u32`
- `object_dirs: list<string>` — the object directories to link, in any order
- `output_dir: string` — required, unlike `compile_model`'s
- `cluster_size?: usize`, `budget?: resource_budget`, `stamp_time?: bool` —
  exactly `compile_model`'s, and with exactly the same meaning
- `lock_path?: string`, `lock_allow_extra?: bool`, `write_lock_path?: string`,
  `lock_sources?: map<unit, string>`, `force_lock?: bool` — the lockfile
  controls (§3.7). All additive-optional; omitting every one of them is the
  link this verb performed before the lockfile existed

Result: a `compile_result` (§3.2), because a link produces the same thing a
compile does. Two fields read differently:

- `objects: list<{unit, object_hash}>` — what was linked, unit-ascending.
  Additive-optional and absent from a `compile_model` envelope: `compile` links
  its units in memory, those objects are never written, and they record no
  `interfaces`, so their `object_hash` would name an artifact nobody holds.
- `stats.source_count` is the number of OBJECTS linked and `stats.chunk_count`
  the chunks they carry. The three entity counts stay `0`: they describe a
  merged authoring view a link does not build.

Stages, in order, with nothing written under `output_dir` unless all three
pass:

1. **Headers only.** No chunk file is opened, so a fault here is reported even
   when every chunk file in every object is unreadable.
2. **Constraint model**, built from the merged headers in the canonical clause
   order.
3. **Emit.** The chunk files copied by hash after being checked twice — against
   the header that names them, and against themselves, since a chunk's address
   is the hash of the entity maps it carries and a body that no longer hashes to
   its own name is refused — then the index, the manifest, the `.ccm`, and the
   provenance sidecars.

For the same model, `link` writes the package `compile` writes, file for file
and byte for byte. That equality is asserted over every scenario pack and every
example by `//compiler:link_oracle_test`.

### 3.7 Lockfile Contract (v1, frozen)

A lockfile pins, per unit, the `object_hash` an integration expects. The
conventional name is `configflux.lock`; any path is accepted. The format is
frozen at `schema_version` 1 (ADR-0058 §D5):

```json
{
  "schema_version": 1,
  "objects": {
    "site_catalogue": { "object_hash": "9f2c...", "source": "" }
  }
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `schema_version` | `u32` | `1`. A file at another version is refused, not read under this one |
| `objects` | `map<unit, entry>` | keyed by unit -- the `package` value every chunk of the unit declares -- and written in unit-name order |
| `objects[].object_hash` | `string` | the `object_hash` this integration expects for that unit |
| `objects[].source` | `string` | optional free text about where the unit came from; absent reads as empty |

A unit key must match `^[a-z]([a-z0-9]|_[a-z0-9])*_?$`, the rule the `package`
field is held to. An unknown key -- at either level -- is refused and named
rather than ignored, because the fields a future version might add are exactly
the ones that would change what a pin means.

**Checked, never fetched.** `source` is informational: no code path reads it,
for any purpose. Objects reach the linker the way build inputs always reach a
build -- a monorepo checkout, a submodule, an artifact store, a CI download --
and the linker's work begins after they have arrived. That is the same boundary
ADR-0032 drew for reference tooling, and it is drawn here deliberately: a pin
format that fetches is a package manager, and a package manager is a supply
chain.

`--lock <path>` runs two checks, both in stage 1 and both before any other
question is asked of the linked set:

1. Every linked object's unit is pinned, at the hash being linked
   (`E_LINK_LOCK_MISMATCH`, naming the unit, the pinned hash and the linked
   hash). A linked unit the lock does not mention at all fails this check too.
2. Every pinned unit was linked (`E_LINK_LOCK_UNLINKED`, naming the unit),
   unless `--lock-allow-extra` declares the subset link deliberate. That flag
   never waives check 1.

`--write-lock <path>` writes the lock after a successful link, so the file
records a set that was actually built. The bytes are a function of the linked
objects alone: `--object` order, argument spelling and object location cannot
reach one. `--lock-source <unit>=<text>` records a note for one unit; a note
for a unit that was not linked has nothing to attach to and is dropped. An
existing file whose contents differ is left alone and the run is refused
(`E_LINK_LOCK_MISMATCH`) unless `--force-lock` is given; because the link
itself succeeded, the package under `--out` is written either way.

## 4) Compiler -> Loader Handoff (CMP)

Emitted CMP contents:
- `cmp.manifest.json`
- `index.cfir.json`
- `chunk-<hash>.cfir` files

`cmp.manifest.json` shape (`CmpManifest`):
- `schema_version: 1`
- `model_hash: string`
- `ir_format_version: 4`
- `index_ref: string` (default `index.cfir.json`)
- `chunk_set_ref: string` (default `.`)
- `config_hash: string`
- `hash_algo: "sha256"`
- `canonicalization_version: 3`
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
- `model_handle?: { model_hash, cmp_manifest_ref, index_ref, chunk_set_ref, ccm_ref }`
- `error_count: u32`
- `warning_count: u32`
- `diagnostics_ref?: string`
- `diagnostics: diagnostics_report`

`model_handle.ccm_ref` (additive optional; ADR-0030) — path to the sibling
`.ccm` solver-model artifact emitted alongside the CMP package, resolved as
`<cmp_manifest_dir>/ccm`, mirroring how `index_ref` and `chunk_set_ref` resolve
relative to the manifest directory. It is how downstream callers locate the
`.ccm` the solver loads, and a usable `.ccm` is a hard precondition for the
selection, resolution, and runtime-open paths — see
`E_SELECTION_SOLVER_MODEL_UNAVAILABLE` (§5.2),
`E_RESOLVE_SOLVER_MODEL_UNAVAILABLE` (§5.3), and
`E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE` (§5.6). `open_model` advertises the
path unconditionally, so a populated `ccm_ref` is not by itself evidence that
the artifact exists or is loadable; those consumers verify it and fail closed.
The field is optional on the wire: it is absent from handles produced before it
existed, and an empty value means "no sibling `.ccm` advertised".

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

`selection_state` admissibility (ADR-0030 Amendment 2):

Every operation that takes a `selection_state` screens the assignments it
already carries against the model, before adjudicating anything. This is
separate from the integrity bindings above, which say the state is coherent and
sealed against this model and say nothing about whether the model has the facets
it names.

"Before adjudicating anything" includes the requests `apply_selection` can
answer from the state alone: a `selection_delta` repeating a choice the state
already holds, one contradicting a context tag, and one re-deciding a facet
already chosen. Over an inadmissible state all three are refused with the
screen's diagnostic rather than answered — a repeated choice is not accepted as
a no-op, and a conflict is not reported about a deployment that cannot exist.

- `choices` — the facet must be one the model knows (`E_SELECTION_UNKNOWN_FACET`
  otherwise) and the option must be in that facet's domain
  (`E_SELECTION_INVALID_OPTION` otherwise). This is the same rule
  `apply_selection` applies to a `selection_delta`, so every choice a client
  built by calling `apply_selection` already satisfies it.
- `context_tags` — screened only on a DECLARED CLOSED facet (bindings included):
  the value must be one of that facet's declared values, else
  `E_SELECTION_INVALID_OPTION`. A tag on an open facet, on a facet only a
  condition mentions, or on one the model does not mention at all is the
  deployment's business and is not screened.
- Every offending entry is reported, choices first and then tags, each in sorted
  order.

`E_SELECTION_CONFLICT` is also what the composition layer reports when the
solver refuses a selection the compiler cannot see through — a contradiction
running through a binding whose value comes from a `derive` table leaves that
binding unbound, and an unbound side makes a constraint evaluate `Unknown`,
which is never a violation. `select`, `options`, `explain` and `resolve` all
report such a selection as a conflict; none of them answers over it.

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
- `implied_choices: map<string, string>` (ADR-0057 §D6) — what the solver
  already determined the constraints decide, supplied by
  `session_compose::resolve`. A compiler-direct caller may pass it too, and
  every entry is screened against the model before it reaches constraint
  evaluation: the key must name a facet or a binding the model declares
  (`E_SELECTION_UNKNOWN_FACET`) and the value must be a member of that facet's
  declared domain, which for a binding is its catalogue's entry ids
  (`E_SELECTION_INVALID_OPTION`). One diagnostic per offending entry, in sorted
  key order. Passing nothing is the empty map and changes nothing.

`resolve_result`:
- `schema_version`
- `status`
- `model_hash`
- `scope`
- `selection_state_hash`
- `resolve_hash?`
- `resolved_output_hash?` (additive optional; ADR-0059 D3) — hash of the
  resolved payload alone; deliberately independent of `model_hash` and of the
  selection, so it changes only when the bytes a consumer receives change.
  Present iff `resolved_output` is present. Adding it does NOT bump
  `schema_version`, and it is outside every hash pre-image — including
  `resolve_hash`'s, which is byte-unchanged by this field.
- `resolved_output?: map<string, resolved_config>` — each `resolved_config`
  carries `package`, `version` and `components`; each component carries `type`,
  `params`, and (ADR-0057 §D7, schema version 5) `requires:
  map<slot, {binding, entry, fields: map<field_id, value>}>` holding the
  catalogue entry each requirement resolved to. `requires` is **omitted** for a
  component that declares no requirement, which is what keeps the payload
  byte-identical for models that do not use the feature. It is inside
  `resolved_output` and therefore already inside the `resolve_hash` and
  `resolved_output_hash` pre-images; no new pre-image field was added.
- `context_tags: map<string, string>`
- `choices: map<string, string>`
- `defaulted_choices: map<string, string>` (auto-bound declared-facet default
  provenance; omitted when empty — ADR-0047 §5). It **is** inside the
  `resolve_hash` pre-image.
- `implied_choices: map<string, string>` (solver-inferred binding provenance;
  omitted when empty — ADR-0057 §D6) — the declared CLOSED facets whose domain
  had collapsed to a single admissible value once the context tags and choices
  were applied. It **is** inside the `resolve_hash` pre-image. A facet recorded
  here is never also recorded in `defaulted_choices`.
- `closed_facet_domains: map<string, list<string>>` (additive optional;
  ADR-0060 D8.1) — the declared values of every CLOSED facet, omitted when
  empty. Open facets are absent rather than flagged: nothing may be entailed
  about an open domain. Recorded by the resolver because it is the last party
  holding a model handle, and forwarded to `runtime_open` so the runtime can
  attribute a rejection whose core mentions a closed facet only negatively.
  Adding it does NOT bump `schema_version`, and it is **outside every hash
  pre-image** — in particular `resolve_hash`'s, which is byte-unchanged by this
  field, and into which it must never be folded: it projects static
  declarations `model_hash` already covers rather than provenance about this
  resolve's selection.
- `resolved_component_dependencies: map<string, map<string, list<string>>>`
- `resolved_artifacts: map<string, artifact_core>`
- `error_count`
- `warning_count`
- `diagnostics_ref?`
- `diagnostics`

`resolve_hash` canonicalization:
- sha256 over canonical JSON payload:
  - `schema_version` (= 5)
  - `model_hash`
  - `scope`
  - canonical `selection_state` tuple
  - canonicalized `resolved_output` (object keys sorted lexicographically at every depth)
  - `defaulted_choices` (auto-bound default provenance; omitted when empty — ADR-0047 §5)
  - `implied_choices` (solver-inferred binding provenance; appended last, omitted when empty — ADR-0057 §D6). Its POSITION is part of the contract: `runtime_api` carries an independent duplicate of this recipe, cross-validated at `runtime_open`, so the field must occupy the same slot in both.

`resolved_output_hash` canonicalization (ADR-0059 D3):
- sha256 over canonical JSON payload, in exactly this field order:
  - `schema_version` (= 5)
  - `scope`
  - canonicalized `resolved_output` (object keys sorted lexicographically at every depth)
- DELIBERATELY excludes `model_hash`, `selection_state`, `context_tags`,
  `choices`, and `defaulted_choices`. Two resolves of two DIFFERENT models that
  deliver a byte-identical payload for the same scope agree on
  `resolved_output_hash` and differ on `resolve_hash` — which is what makes
  "did this change touch that deployment" answerable by hash comparison.

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
- `defaulted_choices` (optional; folds into the `resolve_hash` recompute — a
  projection that drops it fails a defaulted model's open with
  `E_RUNTIME_HASH_MISMATCH`, ADR-0047 §5)
- `implied_choices` (optional; folds into the `resolve_hash` recompute — a
  projection that drops it fails an inferring model's open with
  `E_RUNTIME_HASH_MISMATCH`, ADR-0057 §D6)
- `closed_facet_domains` (optional; ADR-0060 D2) — copied from
  `resolve_result.closed_facet_domains`. Outside every hash pre-image. Absent
  means the caller supplied none: the open succeeds and the runtime's rejection
  explanations degrade to asserted-only attribution (ADR-0060 D7). A table
  naming a facet or value the bound `.ccm` does not carry FAILS the open with
  `E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN` (D6).

The `resolve_hash` recompute is **unconditional**: every open is
cross-validated, including one whose four provenance maps are all empty. There
is no fast path. An earlier revision skipped the recompute for the all-empty
case, which let a caller bind a pre-existing legitimate `resolve_hash` to a
different `resolved_output` by stripping every map, and let a snapshot carrying
genuine provenance be downgraded the same way. Recomputing over all-empty
provenance costs one SHA-256 and reproduces the loader's hash exactly, because
both recipes skip-serialize `defaulted_choices` and `implied_choices` when they
are empty. A caller that ASSEMBLES a request with no `resolve_result` in
hand — a test fixture, or an integrator building the request field by field —
can obtain the value the open expects from `runtime_api::expected_resolve_hash`.
That function is for those callers ONLY: a caller holding a real
`resolve_result` must keep forwarding `resolve_result.resolve_hash` unchanged,
never a freshly computed one. Stamping every open with a recomputed hash makes
the loader's recipe and the runtime's agree by construction, and the
cross-validation is worth nothing to that integrator once the two recipes are
no longer independent.

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
- `closed_facet_domains` — copied verbatim from the open request, never derived
  and never fabricated. Unlike the `resolve_result` field it is NOT
  skip-if-empty: no snapshot field is, so an unpopulated table serializes as
  `"closed_facet_domains":{}` wherever a snapshot is emitted, including
  `configflux_runtime_session_snapshot_json`.
- `resolved_output`
- `resolved_component_dependencies`
- `resolved_artifacts`

Read API in Loop 10:
- `get_scope_metadata(scope_root)`:
  - result metadata: `{ component_count, parameter_count, artifact_count }`
- `get_parameter(path)`:
  - `path` format: `component.<component_id>.param.<param_key>`
  - payload includes resolved metadata/value plus artifact binding metadata for `type=artifact`.
  - `facet?` — the facet this parameter is the declared handle for (ADR-0064).
    Mirrors `ResolvedParameter.facet`; **absent** for a parameter that declares
    no binding, so every existing read envelope is byte-unchanged.
- `list_parameters(scope_root)`:
  - returns `parameter_paths` sorted lexicographically.

Write validation slice used by Loop 10 mutation tests:
- `set_parameter(path, value)`:
  - writable only when `lifecycle=runtime`.
  - enforces type compatibility.
  - enforces numeric/string limits when configured.
  - enforces artifact-ID existence in `resolved_artifacts` for `type=artifact`.
  - enforces the model's declared constraints only for a parameter that
    declares a `facet` binding (ADR-0064 D5). A parameter that merely shares a
    facet's name is not that facet's handle and is not constraint-checked; the
    checks above still apply to it unchanged.

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
  returned; the boundary status remains `Ok`. The same code also fails the open
  closed when the `.ccm` loads but is bound to a different model than the
  snapshot's `model_hash` — a loadable artifact is not by itself the right one,
  and a session that decided its writes against another model would report a
  lineage it never used; the diagnostic names that class and echoes neither
  hash.
- `E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN` — ADR-0060 D6: `runtime-open` fails
  closed when a supplied `closed_facet_domains` table names a facet or value the
  bound `.ccm` symbol table does not carry, because the table then describes a
  different model than the one that will decide this session's writes. Enforced
  at the same shared point as the `.ccm` precondition above, so it applies to
  both open entrypoints, and reported the same way on the C ABI surface (in the
  envelope, boundary status `Ok`, no session handle). An **absent** table is not
  an error: it degrades to asserted-only attribution (D7). The check verifies
  that each supplied facet and value exists in the bound model; the symbol table
  carries no cardinality, so it cannot verify that a facet is closed.
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

### 5.8 Frozen Diagnostic Codes

- `E_LOADER_MANIFEST_INVALID`
- `E_LOADER_MANIFEST_INCONSISTENT`
- `E_LOADER_INDEX_INVALID`
- `E_LOADER_UNSUPPORTED_SCHEMA_VERSION`
- `E_SELECTION_STATE_INVALID`
- `E_SELECTION_UNKNOWN_FACET`
- `E_SELECTION_INVALID_OPTION`
- `E_SELECTION_CONFLICT`
- `E_SELECTION_UNSATISFIABLE`
- `E_SELECTION_ENGINE_DIVERGENCE`
- `E_SELECTION_SOLVER_MODEL_UNAVAILABLE`
- `E_RESOLVE_MODEL_INVALID`
- `E_RESOLVE_SCOPE_INVALID`
- `E_RESOLVE_CONTEXT_UNSATISFIED`
- `E_RESOLVE_FACET_UNBOUND`
- `E_RESOLVE_SOLVER_MODEL_UNAVAILABLE`
- `E_RESOLVE_FAILED`
- `E_EXPORT_RESOLVE_INVALID`
- `E_EXPORT_PROFILE_INVALID`
- `E_EXPORT_ARTIFACT_INVALID`
- `E_EXPORT_SYMBOL_INVALID`
- `E_EXPORT_FAILED`
- `E_SBOM_RESOLVE_INVALID`
- `E_SBOM_PROFILE_INVALID`
- `E_SBOM_ARTIFACT_INVALID`
- `E_SBOM_BINDING_INVALID`
- `E_SBOM_PATH_INVALID`
- `E_SBOM_HASH_INVALID`
- `E_SBOM_STATS_INVALID`
- `E_SBOM_FAILED`
- `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION`
- `E_RUNTIME_OPEN_INVALID`
- `E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN`
- `E_RUNTIME_OPEN_SOLVER_MODEL_UNAVAILABLE`
- `E_RUNTIME_ARTIFACT_UNKNOWN`
- `E_RUNTIME_HASH_MISMATCH`
- `E_RUNTIME_UNKNOWN_SCOPE`
- `E_RUNTIME_UNKNOWN_PATH`
- `E_RUNTIME_TYPE_MISMATCH`
- `E_RUNTIME_LIFECYCLE_IMMUTABLE`
- `E_RUNTIME_LIMIT_VIOLATION`
- `E_RUNTIME_DIRTY_INVALID`
- `E_RUNTIME_EVENT_INVALID`
- `E_RUNTIME_AUDIT_INVALID`
- `E_RUNTIME_COMMIT_INVALID`
- `E_RUNTIME_COMMIT_BASE_MISMATCH`
- `E_RUNTIME_COMMIT_TARGET_HASH_MISMATCH`
- `E_RUNTIME_SYNC_INVALID`
- `E_RUNTIME_SYNC_BASE_MISMATCH`
- `E_RUNTIME_SYNC_BEFORE_HASH_MISMATCH`
- `E_RUNTIME_SYNC_TARGET_HASH_MISMATCH`
- `E_RUNTIME_SYNC_FULL_SNAPSHOT_REQUIRED`
- `E_RUNTIME_SYNC_CONFLICT_OVERRIDDEN`

The list above is the membership contract for §5: every code that can reach a
caller of the loader API or of the runtime delivery slice (§5.6) appears in it,
and nothing else does. §5.1-§5.7 say when each one is raised; this section says
which ones exist. The first thirty are declared by
`compiler/src/loader_api/contracts.rs`, the `E_RUNTIME_*` codes by
`compiler/src/runtime_api/contracts.rs`; the two sets are disjoint.

§5.7's five `E_RUNTIME_CLI_*` codes are deliberately absent. They are the CLI
binary's transport diagnostics, raised before a request reaches the runtime API
at all, and §5.7 freezes them on their own.

`docs/diagnostics.md` is the generated per-code registry, carrying a cause and a
remedy for every code the compiler, interpreter and runtime emit; the frozen
list above is its loader and runtime subset.

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

## 8) Presentation CLI (`cfx`) Envelopes and Exit Codes (not frozen)

`cfx` is the human-facing one-shot CLI over the same loader API (ADR-0042). It
invents no JSON shape of its own except the one below, and adds no diagnostic
namespace: manifest and usage problems are exit-`2` messages, and every
diagnostic it prints is a code the compiler already owns (§3.4, §5.2, §5.3).

Per §7 above, CLI wrappers are not part of the v1 freeze. That is deliberate for
this envelope specifically (ADR-0059 M6): a diff report is a comparison of two
runs and a manifest is a user-side deployment container, so neither belongs in
the resolution contract that every SDK and the C ABI depend on. Placing it in
`cfx` means the tool that emits it can revise it.

### 8.1 `cfx diff` report envelope (`--format json`)

Emitted by `cfx diff` (ADR-0059 D4), one compact JSON document plus a trailing
newline, keys in this order:

- `schema_version` — `PRODUCT_SCHEMA_VERSION` (currently `5`), NOT a private
  `cfx` counter. Every payload-bearing value in the document is
  product-schema-shaped: the `before`/`after` values in `changes[]` are
  fragments of `resolved_output`, whose shape that constant governs, and the
  hashes carry it inside their pre-images. The value therefore moves when the
  product schema moves, including in releases where this envelope's own frame
  did not change; a consumer that only parses the frame can ignore it.
- `base_model_hash`, `head_model_hash` — the two compared models. The
  `--base`/`--head` PATHS are deliberately absent (determinism, §6).
- `cells[]` — one per `(environment, scope)` target, sorted on both axes:
  - `environment`, `scope`
  - `status` — one of `unchanged`, `changed`, `now_unsatisfiable`,
    `now_satisfiable`, `unsatisfiable_both`
  - `base_resolve_hash?`, `head_resolve_hash?`,
    `base_resolved_output_hash?`, `head_resolved_output_hash?` — present for a
    side that resolved. The `unchanged`/`changed` verdict is decided by
    `resolved_output_hash` (§5.3), never by `resolve_hash`.
  - `changes[]` — `{path, field?, kind, before?, after?}`, sorted by path.
    `path` is `component.<id>`, `component.<id>.param.<key>`, `package`,
    `version`, or `selection.defaulted.<facet>`; `field` names the attribute
    that differs (`value`, `type`, `unit`, `safety`, `lifecycle`, `access`,
    `req_id`, `doc`, `limits`) and is absent for an added or removed entry;
    `kind` is `changed`, `added`, or `removed`.
  - `rejections[]` — `{side, code, message}` with `side` one of `base` / `head`,
    carrying the diagnostic that rejected that side.
- `summary` — `{unchanged, changed, now_unsatisfiable, now_satisfiable,
  unsatisfiable_both}` counts.

### 8.2 `cfx` exit codes

The ADR-0042 §3 set `{0, 2, 3}` is widened by one value (ADR-0059 D4):

- `0`: success — and, for `cfx diff`, "no target changed".
- `1`: `cfx diff` only — at least one target is not `unchanged`. This is the
  `diff(1)` / `git diff --exit-code` convention: an expected outcome, not a
  failure, which is what lets the bare command serve as a pull-request check.
  It is unrelated to the interpreter/runtime CLI's `1` (transport/CLI misuse,
  §5.7).
- `2`: usage or IO error.
- `3`: valid input, unsatisfiable selection. `cfx diff` NEVER exits `3` —
  unsatisfiability is a per-target status there, not a command failure.
