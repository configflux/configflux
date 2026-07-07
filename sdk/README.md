# SDK Layout

ConfigFlux SDK and wrapper code must live under `sdk/`, not `runtime/`.

Directories:

1. `sdk/cpp/`: core C++ SDK.
2. `sdk/ros2/`: ROS2 adapter/wrapper and ROS2-specific integration.

Runtime core logic remains in `runtime/` (Rust).

