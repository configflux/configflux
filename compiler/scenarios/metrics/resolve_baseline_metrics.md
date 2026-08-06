# Resolve Baseline Metrics (S1 Smoke)

These baseline numbers are recorded from the resolve path on the smoke profile.

## Commands
- `bazel test //compiler:scenario_smoke_test`
- `bazel test //compiler:scenario_smoke_test --test_output=all --test_arg=--nocapture`

## Baseline Capture
- Date: 2026-02-08
- Scenario: `s1_water_pump`
- Profile: `smoke`
- Host: devcontainer (x86_64 Linux)

## Metrics
- Bazel elapsed wall-clock: `0.351s` (Invocation `7945527b-6614-44e4-b45d-74151826992a`).
- Resolve operation timing (from `resolve_metrics_snapshot`):
  - `resolve_us=632`
- Process memory snapshot:
  - `rss_kib=7936`

These are the resolve baseline values for future resolve-regression tracking.
