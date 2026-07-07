// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <optional>
#include <string>
#include <string_view>
#include <vector>

#include "rcl_interfaces/msg/set_parameters_result.hpp"
#include "rclcpp/rclcpp.hpp"

#include "configflux/ros2/runtime_parameter_adapter.h"

namespace configflux::ros2 {

class RuntimeParameterNodeAdapter {
 public:
  using CallbackHandle =
      rclcpp::node_interfaces::OnSetParametersCallbackHandle::SharedPtr;

  RuntimeParameterNodeAdapter(rclcpp::Node* node,
                              RuntimeParameterAdapter* runtime_adapter);

  AdapterResult DeclareParameters(std::string_view scope_root);

  bool GetParameter(std::string_view ros_parameter_name,
                    rclcpp::Parameter* out_parameter) const;

  CallbackHandle RegisterSetParametersCallback(
      std::string actor = "ros2.set_callback",
      std::optional<std::string> reason = std::nullopt);

  void SetMutationsEnabled(bool enabled);
  bool mutations_enabled() const;

  void ClearSetParametersCallback();

 private:
  rcl_interfaces::msg::SetParametersResult OnSetParameters(
      const std::vector<rclcpp::Parameter>& parameters);

  static bool ToRuntimeParameterValue(const rclcpp::Parameter& parameter,
                                      ParameterValue* out_value);
  static rclcpp::ParameterValue ToRosParameterValue(const ParameterValue& value);
  static std::string ParameterTypeName(rclcpp::ParameterType type);

  rclcpp::Node* node_ = nullptr;
  RuntimeParameterAdapter* runtime_adapter_ = nullptr;
  std::string actor_ = "ros2.set_callback";
  std::optional<std::string> reason_;
  bool mutations_enabled_ = true;
  bool bypass_callback_ = false;
  CallbackHandle callback_handle_;
};

}  // namespace configflux::ros2
