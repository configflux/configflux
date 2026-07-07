# Runtime Use-Case Matrix (RUN-001..RUN-032)

Status: frozen
Date: 2026-02-14

## 1. Coverage Policy
Runtime fixture pack covers:
1. scenarios: `S1`, `S2`, `S3`, `S4`, `S5`
2. profiles: `smoke`, `medium` where available
3. full flow: compiler -> interpreter selection/resolve APIs -> runtime CLI

Scenario/profile IDs used by runtime tests:
1. `s1-smoke`
2. `s1-medium`
3. `s2-smoke`
4. `s2-medium`
5. `s3-smoke`
6. `s3-medium`
7. `s4-smoke`
8. `s4-medium`
9. `s5-smoke`

## 2. Matrix
| ID | Use Case | Expected Result | Scenario/Profile Coverage | Test Evidence |
|---|---|---|---|---|
| RUN-001 | open snapshot happy path | `runtime-open` returns `status=ok` + snapshot | all 9 | `tests::run_001_open_snapshot_happy_path` |
| RUN-002 | open schema version invalid | deterministic `E_RUNTIME_UNSUPPORTED_SCHEMA_VERSION` | representative (S1 smoke) | `tests::run_002_open_snapshot_schema_version_invalid` |
| RUN-003 | open hash mismatch | deterministic `E_RUNTIME_HASH_MISMATCH` | representative (S1 smoke) | `tests::run_003_open_snapshot_hash_mismatch` |
| RUN-004 | open unknown scope root | deterministic `E_RUNTIME_UNKNOWN_SCOPE` | representative (S1 smoke) | `tests::run_004_open_snapshot_unknown_scope_root` |
| RUN-005 | get scope metadata happy | deterministic scope stats | all 9 | `tests::run_005_get_scope_metadata_happy_path` |
| RUN-006 | get scope metadata unknown scope | deterministic `E_RUNTIME_UNKNOWN_SCOPE` | representative (S2 smoke) | `tests::run_006_get_scope_metadata_unknown_scope` |
| RUN-007 | list parameters happy sorted | lexicographically sorted paths | all 9 | `tests::run_007_list_parameters_happy_path_sorted` |
| RUN-008 | get parameter happy | returns expected payload | all 9 | `tests::run_008_get_parameter_happy_path` |
| RUN-009 | get parameter unknown path | deterministic `E_RUNTIME_UNKNOWN_PATH` | representative multi-scenario | `tests::run_009_get_parameter_unknown_path` |
| RUN-010 | set parameter runtime mutable happy | write/readback success | all runtime-mutable profiles (`S1/S2/S4` smoke+medium, `S5` smoke) | `tests::run_010_set_parameter_runtime_mutable_happy_path` |
| RUN-011 | set parameter lifecycle immutable rejected | deterministic `E_RUNTIME_LIFECYCLE_IMMUTABLE` | all 9 | `tests::run_011_set_parameter_lifecycle_immutable_rejected` |
| RUN-012 | set parameter type mismatch rejected | deterministic `E_RUNTIME_TYPE_MISMATCH` | all runtime-mutable profiles (`S1/S2/S4` smoke+medium, `S5` smoke) | `tests::run_012_set_parameter_type_mismatch_rejected` |
| RUN-013 | set parameter limit violation rejected | deterministic `E_RUNTIME_LIMIT_VIOLATION` | all runtime-mutable profiles (`S1/S2/S4` smoke+medium, `S5` smoke) | `tests::run_013_set_parameter_limit_violation_rejected` |
| RUN-014 | set parameter artifact unknown rejected | deterministic `E_RUNTIME_ARTIFACT_UNKNOWN` | all 9 (artifact params forced runtime for mutation) | `tests::run_014_set_parameter_artifact_unknown_rejected` |
| RUN-015 | transport malformed JSON fail-closed | deterministic transport failure, non-leaky stderr | representative | `tests::run_015_transport_malformed_json_fail_closed` |
| RUN-016 | transport oversized payload fail-closed | deterministic `E_RUNTIME_CLI_REQUEST_TOO_LARGE` | representative | `tests::run_016_transport_oversized_payload_fail_closed` |
| RUN-017 | transport request-file I/O failures | deterministic file I/O and malformed-file behavior | representative | `tests::run_017_transport_request_file_io_failures` |
| RUN-018 | transport response-file I/O failures | deterministic `E_RUNTIME_CLI_RESPONSE_IO` | representative | `tests::run_018_transport_response_file_io_failures` |
| RUN-019 | determinism replay read paths byte stable | identical bytes for repeated read requests | all 9 | `tests::run_019_determinism_replay_read_paths_byte_stable` |
| RUN-020 | determinism write/readback + hash lineage | stable lineage (`model_hash`, `selection_state_hash`, `resolve_hash`) + consistent write/readback | all 9 (write assertions where lifecycle=runtime) | `tests::run_020_determinism_write_readback_consistency_and_hash_lineage` |
| RUN-021 | dirty metadata + scheduler arming | write path creates actor/reason/timestamp/deadline/generation metadata and scheduler entries | all runtime-mutable profiles (`S1/S2/S4` smoke+medium, `S5` smoke) | `tests::run_021_set_parameter_populates_dirty_metadata_and_scheduler_entries` |
| RUN-022 | scheduler restart + generation race safety | stale generation timeout is ignored, current generation timeout resets dirty entry after restart | representative (S1 smoke) | `tests::run_022_runtime_open_scheduler_is_generation_safe_and_restart_persistent` |
| RUN-023 | event bus sequence + bounded backpressure | monotonic sequence ordering with bounded buffer and deterministic drop accounting | representative (S1 smoke) | `tests::run_023_event_bus_sequence_and_backpressure_are_deterministic` |
| RUN-024 | atomic batch write + dirty query surface | `set-parameters-atomically`, `list-dirty-parameters`, and `get-dirty-metadata` return deterministic v2 envelopes | representative (S1 smoke) | `tests::run_024_set_parameters_atomically_and_dirty_metadata_commands` |
| RUN-025 | commit/rollback/identity CLI surface | v2 command names for commit, rollback, and identity are wired with deterministic exit behavior | representative (S1 smoke) | `tests::run_025_commit_rollback_and_identity_commands_available` |
| RUN-026 | policy + sync command surface | auto-reset policy set/get and sync check/pull/status commands return stable envelopes and state transitions | representative (S1 smoke) | `tests::run_026_policy_and_sync_commands_happy_path` |
| RUN-027 | audit local-first persistence + idempotent push | audit events accumulate locally, offline push is deferred, online push advances idempotent upload cursor | representative (S1 smoke) | `tests::run_027_audit_pipeline_is_local_first_and_idempotent` |
| RUN-028 | v1 backward compatibility under v2 state | legacy `set-parameter` remains valid with v2 sync/audit state extensions | representative (S1 smoke) | `tests::run_028_backward_compat_v1_set_parameter_with_v2_state_fields` |
| RUN-029 | delta-first sync preconditions + upstream-wins conflicts | pull apply enforces base/before-hash checks, leaves unchanged paths untouched, and records conflict warnings/events | representative (S1 smoke) | `tests::run_029_pull_updates_enforces_delta_preconditions_and_upstream_wins_conflicts` |
| RUN-030 | direct-push offline + reconciliation export | direct push applies while backend is offline and exports deterministic pending reconciliation bundle | representative (S1 smoke) | `tests::run_030_direct_push_offline_and_export_pending_sync_bundle` |
| RUN-031 | malformed runtime event-bus persistence state rejected | malformed persisted event-bus fields fail closed with deterministic runtime event diagnostics | representative (S1 smoke) | `tests::run_031_malformed_state_event_bus_is_rejected_deterministically` |
| RUN-032 | corrupted runtime audit persistence state rejected | corrupted persisted audit sequencing fails closed with deterministic audit diagnostics | representative (S1 smoke) | `tests::run_032_corrupted_persistence_audit_state_is_rejected_deterministically` |

## 3. Gate Targets
1. Full matrix: `bazel test //runtime:runtime_cli_test`
2. Smoke cadence: `bazel test //runtime:runtime_scenario_smoke_test`
3. Medium cadence: `bazel test //runtime:runtime_scenario_medium_test`
4. Full system e2e gate: `bazel test //runtime:runtime_system_e2e_gate_test`
5. Runtime v2 focused gate: `bazel test //runtime:runtime_v2_gate_test`
