# ROS2 SDK

This directory hosts the ConfigFlux ROS2 wrapper and ROS2 integration code.

Build policy:

1. ROS2 packages use `ament_cmake` and are built with `colcon`.
2. Bazel orchestrates ROS2 builds through `//tools:run_ros2_colcon`.
3. SDK implementation remains outside `runtime/`.

See:

- `docs/ros2-bazel-colcon-guidelines.md`

Implemented package:

- `src/configflux_ros2_sdk`
  - `action/PullUpdates.action`
  - `msg/PullUpdateWrite.msg`
  - `msg/RuntimeControlResult.msg`
  - `srv/CheckForUpdates.srv`
  - `srv/CommitConfiguration.srv`
  - `srv/ExportPendingSyncBundle.srv`
  - `srv/GetSyncStatus.srv`
  - `srv/PushAuditEvents.srv`
  - `srv/RollbackDirty.srv`
  - `include/configflux/ros2/runtime_parameter_path.h`
  - `include/configflux/ros2/runtime_parameter_adapter.h`
  - `include/configflux/ros2/runtime_parameter_node_adapter.h`
  - `include/configflux/ros2/runtime_control_adapter.h`
  - `include/configflux/ros2/runtime_control_node_adapter.h`
  - `include/configflux/ros2/runtime_lifecycle_mode.h`
  - `include/configflux/ros2/runtime_diagnostics_bridge.h`
  - `src/runtime_parameter_adapter.cc`
  - `src/runtime_parameter_node_adapter.cc`
  - `src/runtime_control_adapter.cc`
  - `src/runtime_control_node_adapter.cc`
  - `src/runtime_lifecycle_mode.cc`
  - `src/runtime_diagnostics_bridge.cc`
  - `src/runtime_parameter_path.cc`
  - `test/runtime_parameter_node_adapter_test.cc`
  - `test/runtime_control_node_adapter_test.cc`
  - `test/runtime_lifecycle_mode_test.cc`
  - `CMakeLists.txt`
  - `package.xml`

Local Bazel verification target:

```bash
bazel test //sdk/ros2:runtime_parameter_path_test
bazel test //sdk/ros2:runtime_parameter_adapter_test
bazel test //sdk/ros2:runtime_control_adapter_test
bazel test //sdk/ros2:runtime_diagnostics_bridge_test
```

ROS2 package integration test command:

```bash
bazel run //tools:run_ros2_colcon -- --workspace sdk/ros2 -- --packages-select configflux_ros2_sdk
```
