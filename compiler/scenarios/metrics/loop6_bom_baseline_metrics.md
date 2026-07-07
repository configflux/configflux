# Loop 6 Software BOM Baseline Metrics (Smoke + Medium)

These baseline numbers are recorded from the Loop 6 software-BOM export path.

## Commands
- `bazel test //compiler:scenario_smoke_test`
- `bazel test //compiler:scenario_smoke_test --test_output=all --test_arg=--nocapture`

## Baseline Capture
- Date: 2026-02-09
- Scenarios:
  - Smoke: `s1_water_pump`, `s2_wind_turbine`, `s3_automation_cell`, `s4_mobile_robot`
  - Medium validation path: `s1_water_pump`, `s3_automation_cell`
- Host: devcontainer (x86_64 Linux)

## Metrics
- Bazel elapsed wall-clock: `0.465s` (Invocation `1d0fe83c-cfa6-475d-afac-063c00421879`).
- Software BOM operation timings (from `loop6_bom_metrics_snapshot_smoke_and_medium`):
  - `smoke_export_us=755`
  - `medium_s1_export_us=632`
  - `medium_s3_export_us=512`
- Process memory snapshot:
  - `rss_kib=8576`

These are the Loop 6 baseline values for future BOM-regression tracking.
