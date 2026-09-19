# 04 — Fleet Edge Node

A fleet edge-node model that demonstrates the **full ConfigFlux pipeline**
end to end: compile → interpret → resolve → runtime handoff.

Unlike examples 01–02, which stop after the compiler, this example also
exercises the `configflux-interpreter` and `configflux-runtime` binaries
and shows how a resolved configuration is handed off from the interpreter
to the runtime.

## What it shows

- **Full pipeline**: `configflux-compiler` → `configflux-interpreter`
  → `configflux-runtime`
- **3 selection facets** driving overrides across multiple components:
  - `device_class` → picks the firmware artifact and the watchdog timeout
  - `update_channel` → picks the update endpoint and poll interval
  - `region` → gates the `regional_compliance` conditional component
- **Component dependency tree**:
  `network_stack` → `update_agent` → `runtime_tuner`, with `watchdog`
  as a parallel branch that `runtime_tuner` also depends on, and
  `regional_compliance` as a region-conditional sibling
- **Runtime handoff**: the interpreter `resolve` result is reshaped into a
  `runtime-open` request. The resulting `runtime_snapshot` anchors
  `get-scope-metadata` and `list-parameters` reads — all bound to the
  same `model_hash` + `resolve_hash` lineage.

## Files

| File | Purpose |
|------|---------|
| `cue/00_definitions.cue` | 6 parameter definitions, including one artifact slot and one runtime-tunable log level |
| `cue/10_components.cue` | 2 firmware artifacts and 5 components with overrides + a region-conditional component |
| `00_definitions.json`, `10_components.json` | Inheritance-resolved chunks exported from `cue/`; fed to the compiler |
| `run.sh` | Runs compile → verify → interpret (open, init, 3× select, resolve) → runtime (open, get-scope-metadata, list-parameters) |

## Run it

```bash
# Build the three binaries once:
bazel build //compiler //interpreter //runtime

# Then run the example:
./run.sh
```

The script requires `jq` for reshaping JSON between pipeline stages.

## What to look at

After running, check `out/`:

- `compile_result.json` — compilation status and model hash
- `verify_report.json` — graph integrity check
- `open.result.json` — interpreter model handle
- `init.result.json`, `select_*.result.json` — selection state progression
- `resolve.result.json` — the 100% resolved configuration
- `runtime_open.result.json` — runtime snapshot (lineage-bound to resolve)
- `get_scope_metadata.result.json` — component / parameter / artifact counts
- `list_parameters.result.json` — canonical parameter paths under the scope

## Key concepts

**Runtime handoff** means the interpreter produces a resolve result and
the runtime binary opens that result as its starting snapshot. The
runtime `runtime-open` request is a direct projection of the interpreter
`resolve` response:

```jq
{schema_version: 5,
 model_hash: .model_hash,
 resolve_hash: .resolve_hash,
 scope: .scope,
 resolved_output: .resolved_output,
 resolved_component_dependencies: (.resolved_component_dependencies // {}),
 resolved_artifacts: (.resolved_artifacts // {}),
 context_tags: (.context_tags // {}),
 choices: (.choices // {}),
 defaulted_choices: (.defaulted_choices // {}),
 implied_choices: (.implied_choices // {}),
 closed_facet_domains: (.closed_facet_domains // {})}
```

All other `RuntimeOpenRequest` fields default to empty / policy-default
values, so a minimal projection is sufficient for a fresh snapshot. Three
keys in the block above are not optional in practice:

- `defaulted_choices` folds into the `resolve_hash` pre-image, and
  `runtime-open` recomputes that hash. All three of this example's facets
  are closed with declared defaults, so a projection that drops the key
  fails the open with `E_RUNTIME_HASH_MISMATCH`.
- `implied_choices` folds into the same pre-image. It records the facets
  the model's constraints already decided, so a model that leaves nothing
  to infer emits no key and the projection is unchanged — but a projection
  that drops it on a model that does infer fails the open the same way.
- `closed_facet_domains` carries the model's closed facet declarations. A
  device holds a compiled model and a resolve result, never the model
  sources, so this is the only channel by which the runtime learns which
  facets are exhaustive. Drop it and a write rejected by a policy over a
  two-valued closed facet reports the model as over-constrained instead of
  naming the constraint it broke. The open still succeeds — a missing
  diagnostic input is not a failure — but the message quality degrades.
  A table naming a facet or value the compiled model does not carry is a
  different case and fails the open with
  `E_RUNTIME_OPEN_FACET_DOMAIN_UNKNOWN`.

**Region-based conditional component**: `regional_compliance` declares
`condition = "region == 'eu'"`. The compiler includes it in the model
only when the selection context has `region = eu`. It is a sibling to
the main pipeline tree and does not appear in the scope-restricted
`resolved_output`, but it is visible in the full compiled model via
`inspect summary`.

**Hash lineage continuity**: the runtime snapshot carries the same
`model_hash` and `resolve_hash` as the interpreter resolve result.
Subsequent runtime reads preserve both hashes, which lets operators
audit that runtime state is bound to a specific resolved
configuration.

## Next steps

See the [examples index](../README.md) for the full list.
