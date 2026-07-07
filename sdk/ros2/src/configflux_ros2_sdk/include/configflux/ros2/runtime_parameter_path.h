// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <string>
#include <string_view>

namespace configflux::ros2::parameter_path {

bool RuntimePathToRosName(std::string_view runtime_path, std::string* ros_name);
bool RosNameToRuntimePath(std::string_view ros_name, std::string* runtime_path);
bool NormalizeRuntimePath(std::string_view input_path, std::string* runtime_path);

}  // namespace configflux::ros2::parameter_path
