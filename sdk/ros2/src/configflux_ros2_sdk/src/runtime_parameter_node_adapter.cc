// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_parameter_node_adapter.h"

#include <exception>
#include <sstream>
#include <utility>

namespace configflux::ros2 {

namespace {

AdapterResult BuildAdapterError(sdk::RuntimeSdkStatus status,
                                std::string reason) {
  AdapterResult result;
  result.status = status;
  result.successful = false;
  result.reason = std::move(reason);
  return result;
}

std::string JoinRejectedNames(const std::vector<std::string>& names) {
  std::ostringstream stream;
  for (size_t idx = 0; idx < names.size(); ++idx) {
    if (idx != 0) {
      stream << ", ";
    }
    stream << names[idx];
  }
  return stream.str();
}

class ScopedCallbackBypass {
 public:
  explicit ScopedCallbackBypass(bool* bypass_flag) : bypass_flag_(bypass_flag) {
    if (bypass_flag_ != nullptr) {
      *bypass_flag_ = true;
    }
  }

  ~ScopedCallbackBypass() {
    if (bypass_flag_ != nullptr) {
      *bypass_flag_ = false;
    }
  }

  ScopedCallbackBypass(const ScopedCallbackBypass&) = delete;
  ScopedCallbackBypass& operator=(const ScopedCallbackBypass&) = delete;

 private:
  bool* bypass_flag_ = nullptr;
};

}  // namespace

RuntimeParameterNodeAdapter::RuntimeParameterNodeAdapter(
    rclcpp::Node* node, RuntimeParameterAdapter* runtime_adapter)
    : node_(node), runtime_adapter_(runtime_adapter) {}

AdapterResult RuntimeParameterNodeAdapter::DeclareParameters(
    std::string_view scope_root) {
  if (node_ == nullptr) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument,
                             "node is null");
  }
  if (runtime_adapter_ == nullptr) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument,
                             "runtime adapter is null");
  }

  std::vector<RuntimeBackedParameter> runtime_parameters;
  AdapterResult read_result =
      runtime_adapter_->ReadParameters(scope_root, &runtime_parameters);
  if (read_result.status != sdk::RuntimeSdkStatus::kOk ||
      !read_result.successful) {
    return read_result;
  }

  ScopedCallbackBypass callback_bypass(&bypass_callback_);
  try {
    for (const RuntimeBackedParameter& runtime_parameter : runtime_parameters) {
      const rclcpp::ParameterValue value =
          ToRosParameterValue(runtime_parameter.value);
      if (node_->has_parameter(runtime_parameter.ros_parameter_name)) {
        auto set_result = node_->set_parameter(
            rclcpp::Parameter(runtime_parameter.ros_parameter_name, value));
        if (!set_result.successful) {
          return BuildAdapterError(
              sdk::RuntimeSdkStatus::kInvalidArgument,
              "node rejected runtime parameter '" +
                  runtime_parameter.ros_parameter_name + "': " +
                  set_result.reason);
        }
        continue;
      }
      node_->declare_parameter(runtime_parameter.ros_parameter_name, value);
    }
  } catch (const std::exception& ex) {
    return BuildAdapterError(sdk::RuntimeSdkStatus::kInvalidArgument, ex.what());
  }

  return AdapterResult{};
}

bool RuntimeParameterNodeAdapter::GetParameter(std::string_view ros_parameter_name,
                                               rclcpp::Parameter* out_parameter) const {
  if (node_ == nullptr || out_parameter == nullptr) {
    return false;
  }
  return node_->get_parameter(std::string(ros_parameter_name), *out_parameter);
}

RuntimeParameterNodeAdapter::CallbackHandle
RuntimeParameterNodeAdapter::RegisterSetParametersCallback(
    std::string actor, std::optional<std::string> reason) {
  if (node_ == nullptr || runtime_adapter_ == nullptr) {
    return nullptr;
  }

  actor_ = actor.empty() ? "ros2.set_callback" : std::move(actor);
  reason_ = std::move(reason);
  if (reason_.has_value() && reason_->empty()) {
    reason_.reset();
  }

  if (callback_handle_ == nullptr) {
    callback_handle_ = node_->add_on_set_parameters_callback(
        [this](const std::vector<rclcpp::Parameter>& parameters) {
          return OnSetParameters(parameters);
        });
  }
  return callback_handle_;
}

void RuntimeParameterNodeAdapter::SetMutationsEnabled(bool enabled) {
  mutations_enabled_ = enabled;
}

bool RuntimeParameterNodeAdapter::mutations_enabled() const {
  return mutations_enabled_;
}

void RuntimeParameterNodeAdapter::ClearSetParametersCallback() {
  callback_handle_.reset();
}

rcl_interfaces::msg::SetParametersResult
RuntimeParameterNodeAdapter::OnSetParameters(
    const std::vector<rclcpp::Parameter>& parameters) {
  rcl_interfaces::msg::SetParametersResult result;
  result.successful = true;
  result.reason.clear();

  if (bypass_callback_ || parameters.empty()) {
    return result;
  }
  if (runtime_adapter_ == nullptr) {
    result.successful = false;
    result.reason = "runtime adapter is null";
    return result;
  }
  if (!mutations_enabled_) {
    result.successful = false;
    result.reason = "parameter mutations paused in lifecycle inactive state";
    return result;
  }

  std::vector<ParameterWrite> writes;
  writes.reserve(parameters.size());
  for (const rclcpp::Parameter& parameter : parameters) {
    ParameterValue runtime_value;
    if (!ToRuntimeParameterValue(parameter, &runtime_value)) {
      result.successful = false;
      result.reason =
          "unsupported parameter type for '" + parameter.get_name() + "': " +
          ParameterTypeName(parameter.get_type());
      return result;
    }

    ParameterWrite write;
    write.ros_parameter_name = parameter.get_name();
    write.value = std::move(runtime_value);
    writes.push_back(std::move(write));
  }

  std::optional<std::string_view> reason_view = std::nullopt;
  if (reason_.has_value()) {
    reason_view = std::string_view(*reason_);
  }
  const AtomicSetResult adapter_result =
      runtime_adapter_->SetParametersAtomically(writes, actor_, reason_view);

  if (adapter_result.status != sdk::RuntimeSdkStatus::kOk) {
    result.successful = false;
    result.reason = adapter_result.reason.empty()
                        ? "set_parameters_atomically transport failure"
                        : adapter_result.reason;
    return result;
  }
  if (!adapter_result.successful) {
    result.successful = false;
    if (!adapter_result.rejected_parameter_names.empty()) {
      result.reason = "runtime rejected parameter update(s): " +
                      JoinRejectedNames(adapter_result.rejected_parameter_names);
    } else if (!adapter_result.reason.empty()) {
      result.reason = adapter_result.reason;
    } else {
      result.reason = "runtime rejected parameter update";
    }
  }
  return result;
}

bool RuntimeParameterNodeAdapter::ToRuntimeParameterValue(
    const rclcpp::Parameter& parameter, ParameterValue* out_value) {
  if (out_value == nullptr) {
    return false;
  }

  switch (parameter.get_type()) {
    case rclcpp::ParameterType::PARAMETER_BOOL:
      *out_value = parameter.as_bool();
      return true;
    case rclcpp::ParameterType::PARAMETER_INTEGER:
      *out_value = parameter.as_int();
      return true;
    case rclcpp::ParameterType::PARAMETER_DOUBLE:
      *out_value = parameter.as_double();
      return true;
    case rclcpp::ParameterType::PARAMETER_STRING:
      *out_value = parameter.as_string();
      return true;
    default:
      return false;
  }
}

rclcpp::ParameterValue RuntimeParameterNodeAdapter::ToRosParameterValue(
    const ParameterValue& value) {
  if (const auto* bool_value = std::get_if<bool>(&value)) {
    return rclcpp::ParameterValue(*bool_value);
  }
  if (const auto* int_value = std::get_if<int64_t>(&value)) {
    return rclcpp::ParameterValue(*int_value);
  }
  if (const auto* double_value = std::get_if<double>(&value)) {
    return rclcpp::ParameterValue(*double_value);
  }
  return rclcpp::ParameterValue(std::get<std::string>(value));
}

std::string RuntimeParameterNodeAdapter::ParameterTypeName(
    rclcpp::ParameterType type) {
  switch (type) {
    case rclcpp::ParameterType::PARAMETER_NOT_SET:
      return "not_set";
    case rclcpp::ParameterType::PARAMETER_BOOL:
      return "bool";
    case rclcpp::ParameterType::PARAMETER_INTEGER:
      return "integer";
    case rclcpp::ParameterType::PARAMETER_DOUBLE:
      return "double";
    case rclcpp::ParameterType::PARAMETER_STRING:
      return "string";
    case rclcpp::ParameterType::PARAMETER_BYTE_ARRAY:
      return "byte_array";
    case rclcpp::ParameterType::PARAMETER_BOOL_ARRAY:
      return "bool_array";
    case rclcpp::ParameterType::PARAMETER_INTEGER_ARRAY:
      return "integer_array";
    case rclcpp::ParameterType::PARAMETER_DOUBLE_ARRAY:
      return "double_array";
    case rclcpp::ParameterType::PARAMETER_STRING_ARRAY:
      return "string_array";
    default:
      return "unknown";
  }
}

}  // namespace configflux::ros2
