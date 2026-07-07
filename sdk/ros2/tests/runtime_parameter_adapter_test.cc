// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_parameter_adapter.h"

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

namespace {

using configflux::ros2::ParameterValue;
using configflux::ros2::ParameterWrite;
using configflux::ros2::RuntimeBackedParameter;
using configflux::ros2::RuntimeParameterAdapter;
using configflux::sdk::MakeRuntimeCAbiApi;
using configflux::sdk::RuntimeAbiStatus;
using configflux::sdk::RuntimeAbiVersion;
using configflux::sdk::RuntimeCAbiApi;
using configflux::sdk::RuntimeOperation;
using configflux::sdk::RuntimeSession;
using configflux::sdk::RuntimeSessionHandle;

struct FakeSession {
  int id;
};

struct FakeRuntimeState {
  int next_session_id = 1;
  std::string last_atomic_request;
} g_state;

char* AllocateAbiString(const std::string& value) {
  char* buffer = static_cast<char*>(std::malloc(value.size() + 1));
  if (buffer == nullptr) {
    return nullptr;
  }
  std::memcpy(buffer, value.c_str(), value.size() + 1);
  return buffer;
}

RuntimeAbiVersion FakeAbiVersion() { return RuntimeAbiVersion{1, 0, 0}; }

RuntimeAbiStatus FakeAbiHandshake(uint32_t expected_major, uint32_t expected_minor,
                                  RuntimeAbiVersion* out_version) {
  if (out_version == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_version = FakeAbiVersion();
  if (expected_major != 1 || expected_minor > 0) {
    return RuntimeAbiStatus::kVersionMismatch;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionOpen(const char* runtime_open_request_json,
                                 RuntimeSessionHandle** out_handle,
                                 char** out_response_json) {
  if (runtime_open_request_json == nullptr || out_handle == nullptr ||
      out_response_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_handle = nullptr;
  *out_response_json = nullptr;

  auto* session = new FakeSession;
  session->id = g_state.next_session_id++;
  *out_handle = reinterpret_cast<RuntimeSessionHandle*>(session);
  *out_response_json = AllocateAbiString(R"({"status":"ok"})");
  if (*out_response_json == nullptr) {
    delete session;
    *out_handle = nullptr;
    return RuntimeAbiStatus::kInternalError;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionExecuteJson(RuntimeSessionHandle* handle,
                                        uint32_t operation,
                                        const char* request_json,
                                        char** out_response_json) {
  if (handle == nullptr) {
    return RuntimeAbiStatus::kInvalidHandle;
  }
  if (request_json == nullptr || out_response_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_response_json = nullptr;

  const std::string request(request_json);
  std::string response;

  if (operation == static_cast<uint32_t>(RuntimeOperation::kListParameters)) {
    response =
        R"({"status":"ok","parameter_paths":["component.thermal_control.param.control_driver","component.thermal_control.param.max_rpm"]})";
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kGetParameter)) {
    if (request.find("component.thermal_control.param.control_driver") !=
        std::string::npos) {
      response =
          R"({"status":"ok","parameter":{"path":"component.thermal_control.param.control_driver","type":"string","value":"hydra"}})";
    } else if (request.find("component.thermal_control.param.max_rpm") !=
               std::string::npos) {
      response =
          R"({"status":"ok","parameter":{"path":"component.thermal_control.param.max_rpm","type":"integer","value":4200}})";
    } else {
      response = R"({"status":"error","code":"E_RUNTIME_UNKNOWN_PATH"})";
    }
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kSetParameter)) {
    if (request.find("component.thermal_control.param.max_rpm") !=
            std::string::npos &&
        request.find(R"("value":")") != std::string::npos) {
      response = R"({"status":"error","code":"E_RUNTIME_TYPE_MISMATCH"})";
    } else if (request.find("component.thermal_control.param.max_rpm") !=
               std::string::npos) {
      response =
          R"({"status":"ok","parameter":{"path":"component.thermal_control.param.max_rpm","type":"integer","value":4700}})";
    } else {
      response = R"({"status":"error","code":"E_RUNTIME_UNKNOWN_PATH"})";
    }
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kSetParametersAtomically)) {
    g_state.last_atomic_request = request;
    if (request.find("component.thermal_control.param.invalid_param") !=
        std::string::npos) {
      response =
          R"({"status":"error","applied_count":1,"rejected_paths":["component.thermal_control.param.invalid_param"]})";
    } else {
      response =
          R"({"status":"ok","applied_count":2,"rejected_paths":[]})";
    }
  } else {
    response = R"({"status":"error","code":"E_RUNTIME_UNSUPPORTED_OPERATION"})";
  }

  *out_response_json = AllocateAbiString(response);
  if (*out_response_json == nullptr) {
    return RuntimeAbiStatus::kInternalError;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionSnapshotJson(const RuntimeSessionHandle* handle,
                                         char** out_snapshot_json) {
  if (handle == nullptr) {
    return RuntimeAbiStatus::kInvalidHandle;
  }
  if (out_snapshot_json == nullptr) {
    return RuntimeAbiStatus::kNullPointer;
  }
  *out_snapshot_json = AllocateAbiString(R"({"snapshot":"ok"})");
  if (*out_snapshot_json == nullptr) {
    return RuntimeAbiStatus::kInternalError;
  }
  return RuntimeAbiStatus::kOk;
}

RuntimeAbiStatus FakeSessionClose(RuntimeSessionHandle* handle) {
  if (handle == nullptr) {
    return RuntimeAbiStatus::kInvalidHandle;
  }
  delete reinterpret_cast<FakeSession*>(handle);
  return RuntimeAbiStatus::kOk;
}

void FakeStringFree(char* value) { std::free(value); }

RuntimeCAbiApi FakeApi() {
  return MakeRuntimeCAbiApi(
      &FakeAbiVersion, &FakeAbiHandshake, &FakeSessionOpen,
      &FakeSessionExecuteJson, &FakeSessionSnapshotJson, &FakeSessionClose,
      &FakeStringFree);
}

void ResetFakeState() {
  g_state.next_session_id = 1;
  g_state.last_atomic_request.clear();
}

#define CHECK_TRUE(expr)                                                        \
  do {                                                                          \
    if (!(expr)) {                                                              \
      std::cerr << "check failed at " << __FILE__ << ":" << __LINE__ << ": "   \
                << #expr << std::endl;                                          \
      return false;                                                             \
    }                                                                           \
  } while (false)

#define CHECK_EQ(lhs, rhs) CHECK_TRUE((lhs) == (rhs))

bool TestPathMappingRoundTrip() {
  std::string ros_name;
  CHECK_TRUE(RuntimeParameterAdapter::RuntimePathToRosName(
      "component.thermal_control.param.max_rpm", &ros_name));
  CHECK_EQ(ros_name, "thermal_control.max_rpm");

  std::string runtime_path;
  CHECK_TRUE(RuntimeParameterAdapter::RosNameToRuntimePath(
      "thermal_control.max_rpm", &runtime_path));
  CHECK_EQ(runtime_path, "component.thermal_control.param.max_rpm");

  CHECK_TRUE(!RuntimeParameterAdapter::RuntimePathToRosName(
      "component.thermal_control.max_rpm", &ros_name));
  CHECK_TRUE(
      !RuntimeParameterAdapter::RosNameToRuntimePath("max_rpm", &runtime_path));
  return true;
}

bool TestReadParametersMirrorsRuntimeState() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  RuntimeParameterAdapter adapter(&session);
  std::vector<RuntimeBackedParameter> parameters;
  auto result = adapter.ReadParameters("component:thermal_control", &parameters);

  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_TRUE(result.successful);
  CHECK_EQ(parameters.size(), static_cast<size_t>(2));
  CHECK_EQ(parameters[0].ros_parameter_name, "thermal_control.control_driver");
  CHECK_TRUE(std::holds_alternative<std::string>(parameters[0].value));
  CHECK_EQ(std::get<std::string>(parameters[0].value), "hydra");
  CHECK_EQ(parameters[1].ros_parameter_name, "thermal_control.max_rpm");
  CHECK_TRUE(std::holds_alternative<int64_t>(parameters[1].value));
  CHECK_EQ(std::get<int64_t>(parameters[1].value), 4200);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestSetParameterPropagatesRuntimeValidationFailure() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  RuntimeParameterAdapter adapter(&session);
  RuntimeBackedParameter parameter;

  auto success_result = adapter.SetParameter(
      "thermal_control.max_rpm", ParameterValue{int64_t{4700}}, &parameter);
  CHECK_EQ(success_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_TRUE(success_result.successful);
  CHECK_EQ(parameter.ros_parameter_name, "thermal_control.max_rpm");
  CHECK_TRUE(std::holds_alternative<int64_t>(parameter.value));
  CHECK_EQ(std::get<int64_t>(parameter.value), 4700);

  auto reject_result = adapter.SetParameter(
      "thermal_control.max_rpm", ParameterValue{std::string("fast")}, &parameter);
  CHECK_EQ(reject_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_TRUE(!reject_result.successful);
  CHECK_TRUE(reject_result.reason.find("E_RUNTIME_TYPE_MISMATCH") !=
             std::string::npos);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestSetParametersAtomicallyMapsRejectedPaths() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  RuntimeParameterAdapter adapter(&session);
  std::vector<ParameterWrite> writes = {
      {"thermal_control.max_rpm", ParameterValue{int64_t{4600}}},
      {"thermal_control.invalid_param", ParameterValue{int64_t{1}}},
  };

  auto result =
      adapter.SetParametersAtomically(writes, "ros2.set_callback", "unit-test");
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_TRUE(!result.successful);
  CHECK_EQ(result.applied_count, static_cast<uint32_t>(1));
  CHECK_EQ(result.rejected_parameter_names.size(), static_cast<size_t>(1));
  CHECK_EQ(result.rejected_parameter_names[0], "thermal_control.invalid_param");
  CHECK_TRUE(g_state.last_atomic_request.find(R"("actor":"ros2.set_callback")") !=
             std::string::npos);
  CHECK_TRUE(g_state.last_atomic_request.find(R"("reason":"unit-test")") !=
             std::string::npos);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestSetParametersAtomicallySuccess() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  auto open_result = session.Open(R"({"open":"ok"})");
  CHECK_TRUE(open_result.ok());

  RuntimeParameterAdapter adapter(&session);
  std::vector<ParameterWrite> writes = {
      {"thermal_control.max_rpm", ParameterValue{int64_t{4600}}},
      {"thermal_control.control_driver", ParameterValue{std::string("hydra_v2")}},
  };

  auto result =
      adapter.SetParametersAtomically(writes, "ros2.set_callback", std::nullopt);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_TRUE(result.successful);
  CHECK_EQ(result.applied_count, static_cast<uint32_t>(2));
  CHECK_TRUE(result.rejected_parameter_names.empty());
  CHECK_TRUE(result.reason.empty());

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

}  // namespace

int main() {
  if (!TestPathMappingRoundTrip()) {
    return 1;
  }
  if (!TestReadParametersMirrorsRuntimeState()) {
    return 1;
  }
  if (!TestSetParameterPropagatesRuntimeValidationFailure()) {
    return 1;
  }
  if (!TestSetParametersAtomicallyMapsRejectedPaths()) {
    return 1;
  }
  if (!TestSetParametersAtomicallySuccess()) {
    return 1;
  }
  std::cout << "runtime_parameter_adapter_test passed" << std::endl;
  return 0;
}
