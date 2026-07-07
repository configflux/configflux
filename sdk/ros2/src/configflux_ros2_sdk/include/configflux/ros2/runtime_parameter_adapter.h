// SPDX-License-Identifier: BUSL-1.1

#pragma once

#include <cstdint>
#include <optional>
#include <string>
#include <string_view>
#include <variant>
#include <vector>

#include "configflux/sdk/runtime_session.h"

namespace configflux::ros2 {

using ParameterValue = std::variant<bool, int64_t, double, std::string>;

struct RuntimeBackedParameter {
  std::string runtime_path;
  std::string ros_parameter_name;
  std::string runtime_type;
  ParameterValue value;
};

struct ParameterWrite {
  std::string ros_parameter_name;
  ParameterValue value;
};

struct AdapterResult {
  sdk::RuntimeSdkStatus status = sdk::RuntimeSdkStatus::kOk;
  bool successful = true;
  std::string reason;
};

struct AtomicSetResult {
  sdk::RuntimeSdkStatus status = sdk::RuntimeSdkStatus::kOk;
  bool successful = false;
  uint32_t applied_count = 0;
  std::vector<std::string> rejected_parameter_names;
  std::string reason;
};

class RuntimeParameterAdapter {
 public:
  explicit RuntimeParameterAdapter(sdk::RuntimeSession* session);

  static bool RuntimePathToRosName(std::string_view runtime_path,
                                   std::string* ros_name);
  static bool RosNameToRuntimePath(std::string_view ros_name,
                                   std::string* runtime_path);

  sdk::RuntimeCallResult ListRuntimeParameterNames(
      std::string_view scope_root,
      std::vector<std::string>* out_runtime_paths) const;

  AdapterResult ReadParameters(
      std::string_view scope_root,
      std::vector<RuntimeBackedParameter>* out_parameters) const;

  AdapterResult SetParameter(std::string_view ros_parameter_name,
                             const ParameterValue& value,
                             RuntimeBackedParameter* out_parameter) const;

  AtomicSetResult SetParametersAtomically(
      const std::vector<ParameterWrite>& writes, std::string_view actor,
      std::optional<std::string_view> reason = std::nullopt) const;

 private:
  AdapterResult FetchParameterByRuntimePath(
      std::string_view runtime_path,
      RuntimeBackedParameter* out_parameter) const;

  static std::string ParameterValueToJson(const ParameterValue& value);
  static bool ParseParameterFromGetResponse(std::string_view response_json,
                                            RuntimeBackedParameter* out_parameter,
                                            std::string* failure_reason);
  static bool ParseListParameterPaths(std::string_view response_json,
                                      std::vector<std::string>* out_paths,
                                      std::string* failure_reason);
  static bool ParseStatusIsOk(std::string_view response_json, bool* out_ok);
  static bool ParseAtomicSetSummary(std::string_view response_json,
                                    uint32_t* out_applied_count,
                                    std::vector<std::string>* out_rejected_paths,
                                    std::string* failure_reason);

  sdk::RuntimeSession* session_ = nullptr;
};

}  // namespace configflux::ros2
