# 02 — Sensor Gateway

A sensor gateway model that demonstrates selections, parameter overrides,
and conditional components.

## What it shows

- Splitting definitions and components into separate CUE chunks
- Parameter overrides that change values based on selection context
  (e.g., `bus_type`, `environment`)
- A conditional component (`network_monitor`) that is only included
  when `bus_type == 'ethernet'`
- Component dependencies via `depends_on`

## Files

| File | Purpose |
|------|---------|
| `cue/00_definitions.cue` | Three parameter definitions (protocol, poll interval, buffer depth) |
| `cue/10_components.cue` | Three components with overrides and a condition |
| `00_definitions.json`, `10_components.json` | Inheritance-resolved chunks exported from `cue/`; fed to the compiler |
| `run.sh` | Compiles, verifies, and inspects the model |

## Run it

```bash
# Build the compiler first (once):
bazel build //compiler

# Then run the example:
./run.sh
```

## What to look at

After running, check `out/`:

- `compile_result.json` — confirms 3 components, 3 definitions, 0 artifacts
- `verify_report.json` — graph integrity check
- `inspect_summary.json` — lists all components and definitions
- `inspect_data_logger.json` — shows the `data_logger` component detail,
  including its overrides

## Key concepts

**Overrides** change a parameter's value depending on a selection context.
In `cue/10_components.cue`, `data_logger.poll_interval` defaults to `1000`
but becomes `100` when `environment == 'high_speed'`:

```cue
poll_interval: {
	inherits: "poll_interval_ms"
	value:    1000
	overrides: [
		{condition: "environment == 'high_speed'", value: 100},
	]
}
```

**Conditional components** are included or excluded based on a condition.
The `network_monitor` component only exists when the gateway uses an
ethernet bus:

```cue
network_monitor: {
	type: "module"
	depends_on: ["sensor_bus"]
	condition: "bus_type == 'ethernet'"
}
```

## Next steps

See the [examples index](../README.md) for the full list.
