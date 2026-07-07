# ROS2 Integration Guidelines (Bazel + Colcon)

Status: normative
Scope: ConfigFlux ROS2 wrapper under `sdk/ros2/`

## 1. Build Model

1. ROS2 packages must use `ament_cmake`.
2. ROS2 package compilation is done with `colcon`.
3. Bazel remains the top-level orchestrator and invokes colcon via wrapper tooling.

Reference workflow command:

```bash
bazel run //tools:run_ros2_colcon -- --workspace sdk/ros2
```

## 2. Required ROS2 Package Metadata

Each ROS2 package must include:

1. `package.xml` with `ament_cmake` build type export.
2. `CMakeLists.txt` using `find_package(ament_cmake REQUIRED)` and `ament_package()`.

Minimum `package.xml` expectations:

```xml
<buildtool_depend>ament_cmake</buildtool_depend>
<export>
  <build_type>ament_cmake</build_type>
</export>
```

## 3. Workspace Layout

1. ROS2 workspace root is `sdk/ros2/`.
2. ROS2 packages live under `sdk/ros2/src/`.
3. Keep package boundaries explicit (`configflux_ros2_sdk`, diagnostics bridge, service
   adapters, etc.).

## 4. Toolchain and Environment

1. Use latest supported ROS2 distro for initial implementation.
2. Source ROS2 environment before colcon build.
3. Enforce clang/clang++ usage for first-party C++ compilation where supported by package
   toolchains.
4. Align optional local clang bundle location through
   `CONFIGFLUX_CLANG_TOOLCHAIN_DIR` when using ConfigFlux clang wrappers.

## 5. Bazel Integration Contract

1. Bazel wrapper targets/scripts must:
   - validate workspace path,
   - source ROS2 setup when present,
   - execute deterministic colcon commands.
2. ROS2 build arguments are passed through Bazel wrapper tooling instead of ad-hoc local
   scripts.

## 6. Runtime/API Expectations

1. ROS2 parameter API should remain standard for users.
2. Commit/sync operations are exposed via ROS2 services/actions.
3. Runtime events are bridged to ROS2 diagnostics with deterministic status semantics.
