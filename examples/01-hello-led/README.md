# 01 — Hello LED

The simplest possible ConfigFlux model: one component, one parameter,
no selections.

## What it shows

- Defining a parameter type in `[definitions]`
- Declaring a component with a single parameter that `inherits` from
  that definition
- Running the full compile / verify / inspect pipeline

## Files

| File | Purpose |
|------|---------|
| `cue/config.cue` | Definitions and component, authored in CUE (single chunk) |
| `config.json` | Inheritance-resolved chunk exported from `cue/config.cue`; fed to the compiler |
| `run.sh` | Compiles, verifies, and inspects the model |

## Run it

```bash
# Build the compiler first (once):
bazel build //compiler

# Then run the example:
./run.sh
```

## What to look at

After running, check the `out/` directory:

- `compile_result.json` — compilation status and model hash
- `verify_report.json` — graph integrity check (should be `"status": "ok"`)
- `inspect_summary.json` — lists the one definition and one component

## Next steps

Move on to [02-sensor-gateway](../02-sensor-gateway/) to see selections,
overrides, and conditional components.
