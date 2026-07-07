// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_parameter_node_adapter.h"

#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

#include "gtest/gtest.h"

namespace {

using configflux::ros2::RuntimeParameterAdapter;
using configflux::ros2::RuntimeParameterNodeAdapter;
using configflux::sdk::MakeRuntimeCAbiApi;
using configflux::sdk::RuntimeAbiStatus;
using configflux::sdk::RuntimeAbiVersion;
using configflux::sdk::RuntimeCAbiApi;
using configflux::sdk::RuntimeOperation;
using configflux::sdk::RuntimeSdkStatus;
using configflux::sdk::RuntimeSession;
using configflux::sdk::RuntimeSessionHandle;

struct FakeSession {
  int id = 0;
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
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kSetParametersAtomically)) {
    g_state.last_atomic_request = request;
    if (request.find(R"("value":9000)") != std::string::npos) {
      response =
          R"({"status":"error","applied_count":0,"rejected_paths":["component.thermal_control.param.max_rpm"]})";
    } else {
      response = R"({"status":"ok","applied_count":2,"rejected_paths":[]})";
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

std::shared_ptr<rclcpp::Node> MakeTestNode(const std::string& name) {
  rclcpp::NodeOptions options;
  options.start_parameter_services(false);
  options.start_parameter_event_publisher(false);
  return std::make_shared<rclcpp::Node>(name, options);
}

class RuntimeParameterNodeAdapterTest : public ::testing::Test {
 protected:
  static void SetUpTestSuite() {
    if (rclcpp::ok()) {
      return;
    }
    int argc = 0;
    char** argv = nullptr;
    rclcpp::init(argc, argv);
  }

  static void TearDownTestSuite() {
    if (rclcpp::ok()) {
      rclcpp::shutdown();
    }
  }
};

TEST_F(RuntimeParameterNodeAdapterTest, DeclareAndGetParametersFromRuntime) {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  ASSERT_TRUE(session.Open(R"({"open":"ok"})").ok());

  auto node = MakeTestNode("configflux_runtime_parameter_declare");
  RuntimeParameterAdapter runtime_adapter(&session);
  RuntimeParameterNodeAdapter node_adapter(node.get(), &runtime_adapter);

  const auto declare_result =
      node_adapter.DeclareParameters("component:thermal_control");
  EXPECT_EQ(declare_result.status, RuntimeSdkStatus::kOk);
  EXPECT_TRUE(declare_result.successful) << declare_result.reason;

  rclcpp::Parameter max_rpm;
  ASSERT_TRUE(node_adapter.GetParameter("thermal_control.max_rpm", &max_rpm));
  EXPECT_EQ(max_rpm.get_type(), rclcpp::ParameterType::PARAMETER_INTEGER);
  EXPECT_EQ(max_rpm.as_int(), 4200);

  rclcpp::Parameter control_driver;
  ASSERT_TRUE(
      node_adapter.GetParameter("thermal_control.control_driver", &control_driver));
  EXPECT_EQ(control_driver.get_type(), rclcpp::ParameterType::PARAMETER_STRING);
  EXPECT_EQ(control_driver.as_string(), "hydra");

  EXPECT_EQ(session.Close(), RuntimeSdkStatus::kOk);
}

TEST_F(RuntimeParameterNodeAdapterTest, CallbackForwardsAtomicRuntimeWrites) {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  ASSERT_TRUE(session.Open(R"({"open":"ok"})").ok());

  auto node = MakeTestNode("configflux_runtime_parameter_callback_success");
  RuntimeParameterAdapter runtime_adapter(&session);
  RuntimeParameterNodeAdapter node_adapter(node.get(), &runtime_adapter);

  const auto declare_result =
      node_adapter.DeclareParameters("component:thermal_control");
  ASSERT_EQ(declare_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(declare_result.successful) << declare_result.reason;
  ASSERT_NE(node_adapter.RegisterSetParametersCallback("ros2.node_callback",
                                                       "integration-test"),
            nullptr);

  auto result = node->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{4600}),
       rclcpp::Parameter("thermal_control.control_driver", std::string("hydra_v2"))});
  EXPECT_TRUE(result.successful) << result.reason;
  EXPECT_NE(g_state.last_atomic_request.find(R"("actor":"ros2.node_callback")"),
            std::string::npos);
  EXPECT_NE(g_state.last_atomic_request.find(R"("reason":"integration-test")"),
            std::string::npos);
  EXPECT_NE(g_state.last_atomic_request.find(
                "component.thermal_control.param.max_rpm"),
            std::string::npos);
  EXPECT_NE(g_state.last_atomic_request.find(
                "component.thermal_control.param.control_driver"),
            std::string::npos);

  EXPECT_EQ(session.Close(), RuntimeSdkStatus::kOk);
}

TEST_F(RuntimeParameterNodeAdapterTest, CallbackRespectsLifecycleMutationPause) {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  ASSERT_TRUE(session.Open(R"({"open":"ok"})").ok());

  auto node = MakeTestNode("configflux_runtime_parameter_lifecycle_pause");
  RuntimeParameterAdapter runtime_adapter(&session);
  RuntimeParameterNodeAdapter node_adapter(node.get(), &runtime_adapter);

  const auto declare_result =
      node_adapter.DeclareParameters("component:thermal_control");
  ASSERT_EQ(declare_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(declare_result.successful) << declare_result.reason;
  ASSERT_NE(node_adapter.RegisterSetParametersCallback(), nullptr);

  node_adapter.SetMutationsEnabled(false);
  auto paused_result = node->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{4500})});
  EXPECT_FALSE(paused_result.successful);
  EXPECT_NE(paused_result.reason.find("lifecycle inactive state"),
            std::string::npos);
  EXPECT_EQ(g_state.last_atomic_request, "");

  node_adapter.SetMutationsEnabled(true);
  auto resumed_result = node->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{4500})});
  EXPECT_TRUE(resumed_result.successful) << resumed_result.reason;
  EXPECT_NE(g_state.last_atomic_request.find(
                "component.thermal_control.param.max_rpm"),
            std::string::npos);

  EXPECT_EQ(session.Close(), RuntimeSdkStatus::kOk);
}

TEST_F(RuntimeParameterNodeAdapterTest, CallbackRejectsRuntimeValidationError) {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  ASSERT_TRUE(session.Open(R"({"open":"ok"})").ok());

  auto node = MakeTestNode("configflux_runtime_parameter_callback_reject");
  RuntimeParameterAdapter runtime_adapter(&session);
  RuntimeParameterNodeAdapter node_adapter(node.get(), &runtime_adapter);

  const auto declare_result =
      node_adapter.DeclareParameters("component:thermal_control");
  ASSERT_EQ(declare_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(declare_result.successful) << declare_result.reason;
  ASSERT_NE(node_adapter.RegisterSetParametersCallback(), nullptr);

  auto result = node->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{9000})});
  EXPECT_FALSE(result.successful);
  EXPECT_NE(result.reason.find("thermal_control.max_rpm"), std::string::npos);

  rclcpp::Parameter max_rpm;
  ASSERT_TRUE(node_adapter.GetParameter("thermal_control.max_rpm", &max_rpm));
  EXPECT_EQ(max_rpm.as_int(), 4200);

  EXPECT_EQ(session.Close(), RuntimeSdkStatus::kOk);
}

TEST_F(RuntimeParameterNodeAdapterTest, CallbackRejectsUnsupportedParameterType) {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  ASSERT_TRUE(session.Open(R"({"open":"ok"})").ok());

  auto node = MakeTestNode("configflux_runtime_parameter_callback_types");
  RuntimeParameterAdapter runtime_adapter(&session);
  RuntimeParameterNodeAdapter node_adapter(node.get(), &runtime_adapter);

  const auto declare_result =
      node_adapter.DeclareParameters("component:thermal_control");
  ASSERT_EQ(declare_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(declare_result.successful) << declare_result.reason;
  node->declare_parameter("thermal_control.waveform", std::vector<int64_t>{1, 2, 3});
  ASSERT_TRUE(node->has_parameter("thermal_control.waveform"));
  ASSERT_NE(node_adapter.RegisterSetParametersCallback(), nullptr);

  auto result = node->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.waveform", std::vector<int64_t>{4, 5})});
  EXPECT_FALSE(result.successful);
  EXPECT_NE(result.reason.find("unsupported parameter type"),
            std::string::npos);
  EXPECT_EQ(g_state.last_atomic_request, "");

  EXPECT_EQ(session.Close(), RuntimeSdkStatus::kOk);
}

}  // namespace
