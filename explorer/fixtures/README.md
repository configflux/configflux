# Explorer fixtures — scenario S1 (water pump)

These JSON files are the sample artifacts the model explorer loads: real
ConfigFlux pipeline output for scenario **S1 (water pump)**, carrying the
current product schema version (`schema_version: 4`). The hermetic Bazel test
`//explorer:fixture_schema_test` guards them: it reads
`PRODUCT_SCHEMA_VERSION` from `compiler/src/product_api.rs` rather than restating
it, so bumping the product schema version fails that test until these fixtures
are regenerated (below) and `SUPPORTED_VERSIONS` in `explorer/js/schema.js` is
updated to match.

| File | Pipeline artifact | Explorer view |
|------|-------------------|---------------|
| `s1-model-summary.json` | `compiler inspect … summary` — `InspectionResult` | Model view (inventory tree + `source_digest` header) |
| `s1-facets.json` | `cfx options --format json` — `[GetSelectionOptionsResult]` | Model view (facet → option tree) |
| `s1-resolve-snapshot.json` | `cfx resolve --format json` — `ResolveResult` | Resolution view (params table + hash lineage) |
| `s1-explain-conflict.json` | `cfx explain --format json` — `ExplainRejectionResult` | Explain view (labeled unsat core) |

## Data provenance

Every id, component, parameter, artifact and option value traces to the
committed S1 smoke model and its goldens:

- `compiler/scenarios/s1_water_pump/smoke/cue/00_definitions.json`
- `compiler/scenarios/s1_water_pump/smoke/cue/10_components.json`
- `compiler/scenarios/s1_water_pump/smoke/golden/resolved_output.json`
- `compiler/scenarios/s1_water_pump/smoke/golden/resolve_result.thermal_control.hydra_x200_dual_us.json`

Three of the four fixtures are emitted directly by the commands below and are
byte-identical to their output: `s1-model-summary.json`, `s1-facets.json` and
`s1-explain-conflict.json`. Run those commands from the repository root so the
relative paths resolve. A model's hash covers the content of its sources, not
the paths they were compiled from, so an absolute path or a relocated chunk
yields the same hash; editing the step 6 chunk changes its content, and that
does change the hash in the result. This holds for both hashes these fixtures
carry, and they are different values under different names: step 3 reports a
`source_digest` over the source contents, while steps 4–6 report a `model_hash`
identifying the compiled package.

`s1-explain-conflict.json` shows the Explain view on the S1 cross-brand conflict:
choosing the `aeroflux` brand together with the `x200` model, which is a Hydra
model. The S1 **smoke** model expresses that relationship as an override selector
rather than a rule the solver enforces, so on its own it rejects nothing and
there is no core to explain. Step 6 therefore compiles the same smoke sources
together with a small chunk that declares the two facets the rule names and
states the exclusion directly. The labeled unsat core in the fixture is the
solver's own — including the `constraint_id` naming the rule it came from —
not an illustration of one.

`s1-resolve-snapshot.json` is the one fixture the commands below do **not**
reproduce, and this is deliberate. Its `resolve_hash` (`1f93f566…`) is the
authentic S1 smoke value recorded in
`compiler/tests/fixtures/byte-stability-baselines.json`, which comes from a
`component:thermal_control`-scoped, context-only resolution run in process. Step
5 resolves a different scope with explicit choices, so its `resolve_hash`
differs — the compiled model is the same either way, since a model's hash covers
content alone. The schema guard
pins the fixture to that baseline so the two cannot drift apart. Its `model_hash`
and `selection_state_hash` are deterministic, schema-valid, S1-scoped stand-ins
(sha256 of a stable label). Step 5 re-runs the resolution the Resolution view
demonstrates; expect its hash header to differ from the committed fixture's.

## Regeneration command

Fixtures are regenerated from source so they track schema changes. Run from the
repository root:

```bash
# 1. Build the compiler and the cfx one-shot CLI (Bazel only).
bazel build //compiler //cfx

COMPILER=bazel-bin/compiler/compiler
CFX=bazel-bin/cfx/cfx
S1=compiler/scenarios/s1_water_pump/smoke/cue

# 2. Compile the S1 smoke model into a compiled model package (CMP).
"$COMPILER" compile \
  --source "$S1/00_definitions.json" \
  --source "$S1/10_components.json" \
  --out /tmp/s1-cmp > /dev/null
MODEL=/tmp/s1-cmp/cmp.manifest.json

# 3. Model inventory summary -> s1-model-summary.json
"$COMPILER" inspect \
  --source "$S1/00_definitions.json" \
  --source "$S1/10_components.json" \
  summary > explorer/fixtures/s1-model-summary.json

# 4. Facets and their valid options -> s1-facets.json
#    `cfx` prints compact JSON; the committed fixtures are indented for reading.
"$CFX" options --model "$MODEL" --format json \
  | python3 -m json.tool --indent 2 > explorer/fixtures/s1-facets.json

# 5. Re-run the resolution the Resolution view shows. `region` is a facet of
#    this model, so it must be chosen alongside the other three or the
#    selection is unsatisfiable. This does NOT regenerate
#    s1-resolve-snapshot.json — see "Data provenance" above.
"$CFX" resolve --model "$MODEL" \
  --select cooling_brand=hydra --select cooling_model=x200 \
  --select pump_type=dual --select region=eu \
  --out /tmp/s1-out --format json | python3 -m json.tool --indent 2

# 6. Explain a conflicting selection -> s1-explain-conflict.json
#    The smoke model has no rule forbidding aeroflux + x200, so compile it
#    together with a chunk that states the exclusion. A constraint may only
#    name facets declared under `facets`, and the smoke sources declare none —
#    theirs are inferred from component and override conditions — so the chunk
#    declares the two the rule names, listing the values step 4 reports. They
#    are declared `open` because that is what the smoke model already means: a
#    domain inferred from conditions is never known to be exhaustive, and
#    closing it here would add an exactly-one rule the model never stated.
#    Keep this path as written: the steps below refer to it by name. It no
#    longer affects the model's hash, only the chunk's content does.
cat > /tmp/s1-constraints.json <<'JSON'
{
    "package": "s1_water_pump",
    "version": "1.0.0",
    "facets": {
        "cooling_brand": {
            "values": ["aeroflux", "hydra"],
            "open": true
        },
        "cooling_model": {
            "values": ["a9", "x200"],
            "open": true
        }
    },
    "constraints": {
        "aeroflux_excludes_x200": {
            "condition": "cooling_brand != 'aeroflux' || cooling_model != 'x200'",
            "doc": "The Aeroflux brand does not offer the X200 cooling model."
        }
    }
}
JSON

"$COMPILER" compile \
  --source "$S1/00_definitions.json" \
  --source "$S1/10_components.json" \
  --source /tmp/s1-constraints.json \
  --out /tmp/s1-cmp-constrained > /dev/null

"$CFX" explain --model /tmp/s1-cmp-constrained/cmp.manifest.json \
  --select cooling_brand=aeroflux --select cooling_model=x200 \
  --format json | python3 -m json.tool --indent 2 \
  > explorer/fixtures/s1-explain-conflict.json
```

After regenerating, re-run the schema guard:

```bash
bazel test //explorer:fixture_schema_test
```

If the product schema version changed, update `SUPPORTED_VERSIONS` in
`explorer/js/schema.js` to match. The Python guard needs no edit — it reads the
version from `compiler/src/product_api.rs`.
