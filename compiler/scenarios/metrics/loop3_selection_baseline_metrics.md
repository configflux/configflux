# Loop 3 Selection Baseline Metrics (S1/S3 Smoke)

These baseline numbers are recorded from the Loop 3 guided-selection smoke path.

## Commands
- `bazel test //compiler:scenario_smoke_test`
- `bazel test //compiler:scenario_smoke_test --test_output=all --test_arg=--nocapture`

## Baseline Capture
- Date: 2026-02-08
- Scenarios: `s1_water_pump`, `s3_automation_cell`
- Profile: `smoke`
- Host: devcontainer (x86_64 Linux)

## Metrics
- Bazel elapsed wall-clock: `0.395s` (Invocation `eef2ed49-f407-4d3d-be3a-1341a6243878`).
- Selection operation timings (from `loop3_selection_metrics_snapshot`):
  - `get_options_us=437`
  - `apply_selection_us=453`
  - `explain_rejection_us=441`
- Process memory snapshot:
  - `rss_kib=7296`

These are the Loop 3 baseline values for future smoke-regression tracking.
