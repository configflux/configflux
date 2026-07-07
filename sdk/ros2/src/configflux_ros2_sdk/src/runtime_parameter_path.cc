// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_parameter_path.h"

namespace configflux::ros2::parameter_path {
namespace {

constexpr char kRuntimePathPrefix[] = "component.";
constexpr char kRuntimeParamMarker[] = ".param.";

bool ParseCanonicalRuntimePath(std::string_view runtime_path, size_t* component_start,
                               size_t* marker_pos, size_t* param_start) {
  if (!runtime_path.starts_with(kRuntimePathPrefix)) {
    return false;
  }

  const size_t local_component_start = std::string_view(kRuntimePathPrefix).size();
  const size_t local_marker_pos =
      runtime_path.find(kRuntimeParamMarker, local_component_start);
  if (local_marker_pos == std::string_view::npos ||
      local_marker_pos == local_component_start) {
    return false;
  }

  const size_t local_param_start =
      local_marker_pos + std::string_view(kRuntimeParamMarker).size();
  if (local_param_start >= runtime_path.size()) {
    return false;
  }

  if (component_start != nullptr) {
    *component_start = local_component_start;
  }
  if (marker_pos != nullptr) {
    *marker_pos = local_marker_pos;
  }
  if (param_start != nullptr) {
    *param_start = local_param_start;
  }
  return true;
}

}  // namespace

bool RuntimePathToRosName(std::string_view runtime_path, std::string* ros_name) {
  if (ros_name == nullptr) {
    return false;
  }
  size_t component_start = 0;
  size_t marker_pos = 0;
  size_t param_start = 0;
  if (!ParseCanonicalRuntimePath(runtime_path, &component_start, &marker_pos,
                                 &param_start)) {
    return false;
  }

  *ros_name =
      std::string(runtime_path.substr(component_start, marker_pos - component_start));
  ros_name->push_back('.');
  ros_name->append(runtime_path.substr(param_start));
  return true;
}

bool RosNameToRuntimePath(std::string_view ros_name, std::string* runtime_path) {
  if (runtime_path == nullptr) {
    return false;
  }

  const size_t dot = ros_name.find('.');
  if (dot == std::string_view::npos || dot == 0 || dot + 1 >= ros_name.size()) {
    return false;
  }

  *runtime_path = kRuntimePathPrefix;
  runtime_path->append(ros_name.substr(0, dot));
  runtime_path->append(kRuntimeParamMarker);
  runtime_path->append(ros_name.substr(dot + 1));
  return true;
}

bool NormalizeRuntimePath(std::string_view input_path, std::string* runtime_path) {
  if (runtime_path == nullptr || input_path.empty()) {
    return false;
  }

  if (ParseCanonicalRuntimePath(input_path, nullptr, nullptr, nullptr)) {
    *runtime_path = std::string(input_path);
    return true;
  }
  if (input_path.starts_with(kRuntimePathPrefix)) {
    return false;
  }

  return RosNameToRuntimePath(input_path, runtime_path);
}

}  // namespace configflux::ros2::parameter_path
