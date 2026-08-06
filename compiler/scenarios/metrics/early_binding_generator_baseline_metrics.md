# Early-Binding Generator Baseline Metrics (S1 Smoke)

These baseline numbers are recorded from the early-binding generator path on the smoke profile.

## Commands
- `bazel test //compiler:scenario_smoke_test`
- `bazel test //compiler:scenario_smoke_test --test_output=all --test_arg=--nocapture`

## Baseline Capture
- Date: 2026-02-09
- Scenario: `s1_water_pump`
- Profile: `smoke`
- Host: devcontainer (x86_64 Linux)

## Metrics
- Bazel elapsed wall-clock: `0.563s` (Invocation `3ef27737-f82d-4478-9dc0-8ab2c09452e1`).
- Generator operation timing (from `early_binding_generator_metrics_snapshot` in `--nocapture` run):
  - `export_us=206`
- Process memory snapshot:
  - `rss_kib=7808`

These are the early-binding baseline values for future generation-regression tracking.
