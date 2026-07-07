# Loop 0 Baseline Metrics (S1 Smoke)

These baseline numbers are recorded from the Loop 0 smoke closed-loop command path.

## Command
- `bazel test //compiler:scenario_smoke_test`

## Baseline Capture
- Date: 2026-02-08
- Scenario: `s1_water_pump`
- Profile: `smoke`
- Host: devcontainer (x86_64 Linux)

## Metrics
- Bazel elapsed wall-clock: `0.334s` (Invocation `3bd1fe96-63db-4283-bcd7-f06f9c91f8ec`).
- Closed-loop phase timings (from `loop0_s1_smoke_closed_loop_matches_goldens`):
  - `ingest_us=916`
  - `verify_us=127`
  - `resolve_us=130`
  - `emit_us=1565`
- Process memory snapshot:
  - `rss_kib=6272`

These are the Loop 0 baseline values for future smoke-regression tracking.
