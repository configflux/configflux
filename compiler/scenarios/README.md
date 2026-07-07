# ConfigFlux Scenario Pack

This folder is the Loop 0 verification harness baseline for V1.

## Naming Convention
- Scenario folder: `s<id>_<domain>` in snake_case (examples: `s1_water_pump`, `s4_mobile_robot`).
- Profile folder: `smoke`, `medium`, `large`.
- Chunk file: `<order>_<purpose>.toml` where `<order>` is two digits (`00_`, `10_`, ...).
- Mutation file: snake_case mutation intent (`unknown_dependency.toml`, `unreachable_branch.toml`).
- Golden file: snake_case artifact name (`verify_report.ok.json`, `resolved_output.json`).

## Folder Layout

```text
scenarios/
  s1_water_pump/
    smoke/
      profile.toml
      chunks/
      mutations/
      golden/
  s2_wind_turbine/
    smoke/
  s3_automation_cell/
    smoke/
  s4_mobile_robot/
    smoke/
  s5_building_hvac/
    smoke/
  metrics/
```

## Golden Artifact Format (Loop 0 Baseline)
Each smoke scenario uses these baseline golden outputs:
- `golden/verify_report.ok.json`: expected verification report shape and check results.
- `golden/resolved_output.json`: resolved 100% payload for the default smoke context.
- `golden/emitted_manifest.json`: deterministic summary of emitted IR contents.

Loop 0 keeps this minimal and deterministic so CI can run fast and fail loudly.
