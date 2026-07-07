# Loop 7 Scale Baseline Metrics (Medium + Large)

These baseline numbers are recorded from the Loop 7 medium/large closed-loop path:
`verify -> compile -> selection -> resolve -> export -> software BOM`.

## Commands
- `bazel test //compiler:scenario_smoke_test`
- `bazel test //compiler:scenario_medium_test`
- `bazel test //compiler:scenario_large_test`
- `bazel test //compiler:compiler_test --test_output=all --test_arg=loop7_medium_performance_thresholds_and_metrics_snapshot --test_arg=--nocapture`
- `bazel test //compiler:compiler_test --test_output=all --test_arg=loop7_large_performance_thresholds_and_metrics_snapshot --test_arg=--nocapture`

## Baseline Capture
- Date: 2026-02-09
- Scenarios:
  - Medium: `s1_water_pump`, `s2_wind_turbine`, `s3_automation_cell`, `s4_mobile_robot`
  - Large: `s3_automation_cell`, `s4_mobile_robot`
- Host: devcontainer (x86_64 Linux)

## Metrics
- Medium scale timings (from `loop7_scale_metrics_medium`):
  - `medium_s2_verify_us=1198`
  - `medium_s2_compile_us=1845`
  - `medium_s2_selection_us=2158`
  - `medium_s2_resolve_us=841`
  - `medium_s2_export_us=350`
  - `medium_s2_bom_us=629`
  - `medium_s4_verify_us=2495`
  - `medium_s4_compile_us=4095`
  - `medium_s4_selection_us=6574`
  - `medium_s4_resolve_us=1401`
  - `medium_s4_export_us=431`
  - `medium_s4_bom_us=1220`
  - `rss_kib=7552`
- Large scale timings (from `loop7_scale_metrics_large`):
  - `large_s3_verify_us=132237`
  - `large_s3_compile_us=133538`
  - `large_s3_selection_us=198800`
  - `large_s3_resolve_us=49356`
  - `large_s3_export_us=23931`
  - `large_s3_bom_us=100171`
  - `large_s4_verify_us=127315`
  - `large_s4_compile_us=154532`
  - `large_s4_selection_us=247016`
  - `large_s4_resolve_us=54929`
  - `large_s4_export_us=31794`
  - `large_s4_bom_us=115209`
  - `rss_kib=15844`

## Threshold Policy
Guardrail assertions are encoded in `compiler/src/scenario_loop7_tests.rs`:
- Medium thresholds:
  - `verify_us <= 750000`
  - `compile_us <= 1200000`
  - `selection_us <= 500000`
  - `resolve_us <= 600000`
  - `export_us <= 600000`
  - `bom_us <= 900000`
- Large thresholds:
  - `verify_us <= 2500000`
  - `compile_us <= 3500000`
  - `selection_us <= 1200000`
  - `resolve_us <= 2000000`
  - `export_us <= 2000000`
  - `bom_us <= 2500000`
- Memory threshold:
  - `rss_kib <= 524288`

Tolerance policy: thresholds are intentionally wide (well above current baseline) to catch severe regressions while minimizing CI flakiness in shared CI environments.
