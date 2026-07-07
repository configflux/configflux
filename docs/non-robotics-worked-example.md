# ConfigFlux Non-Robotics Worked Example

The non-robotics scenario (`s5_building_hvac`) follows the same
compile-interpret-runtime pipeline as the
[canonical worked example](canonical-worked-example.md), using a
building HVAC domain instead of water pumps.

## Quick reference

| Property | Value |
|----------|-------|
| Scenario | `compiler/scenarios/s5_building_hvac/smoke/chunks/` |
| Scope | `component:climate_controller` |
| Selection chain | `occupancy_class=hospital`, `filtration_grade=hepa`, `region=us` |
| Resolved components | `air_handler`, `climate_controller`, `power_distribution`, `pressure_monitor` |
| Key artifact parameter | `controller_package = hospital_hepa_us_controller` |

## Running it

Use the same command sequence from the canonical example, substituting
the S5 scenario paths and scope shown above. The export contract
(`config.hpp`, `config_artifact_manifest.json`,
`config_build_flags.cmake`) and runtime handoff flow are identical.

## See also

- [Canonical worked example](canonical-worked-example.md) — full
  pipeline walkthrough
- [`examples/`](../examples/) — runnable progressive examples starting
  from the simplest possible model
