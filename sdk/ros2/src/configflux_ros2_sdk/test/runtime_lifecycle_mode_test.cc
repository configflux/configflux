// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_control_adapter.h"
#include "configflux/ros2/runtime_control_node_adapter.h"
#include "configflux/ros2/runtime_lifecycle_mode.h"
#include "configflux/ros2/runtime_parameter_adapter.h"
#include "configflux/ros2/runtime_parameter_node_adapter.h"

#include <chrono>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>

#include "gtest/gtest.h"
#include "rclcpp/rclcpp.hpp"

namespace {

using configflux::ros2::RuntimeControlAdapter;
using configflux::ros2::RuntimeControlNodeAdapter;
using configflux::ros2::RuntimeLifecycleConfigureRequest;
using configflux::ros2::RuntimeLifecycleMode;
using configflux::ros2::RuntimeLifecycleState;
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
  uint32_t last_operation = 0;
  std::string last_request_json;
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

  g_state.last_operation = operation;
  g_state.last_request_json = request_json;

  std::string response;
  if (operation == static_cast<uint32_t>(RuntimeOperation::kListParameters)) {
    response =
        R"({"status":"ok","parameter_paths":["component.thermal_control.param.control_driver","component.thermal_control.param.max_rpm"]})";
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kGetParameter)) {
    if (g_state.last_request_json.find("component.thermal_control.param.control_driver") !=
        std::string::npos) {
      response =
          R"({"status":"ok","parameter":{"path":"component.thermal_control.param.control_driver","type":"string","value":"hydra"}})";
    } else {
      response =
          R"({"status":"ok","parameter":{"path":"component.thermal_control.param.max_rpm","type":"integer","value":4200}})";
    }
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kSetParametersAtomically)) {
    g_state.last_atomic_request = request_json;
    response = R"({"status":"ok","applied_count":1,"rejected_paths":[]})";
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kCommitConfiguration)) {
    response = R"({"status":"ok","commit_id":"commit-1"})";
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kGetSyncStatus)) {
    response = R"({"status":"ok","sync_status":{"sync_state":"idle"}})";
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
  g_state.last_operation = 0;
  g_state.last_request_json.clear();
  g_state.last_atomic_request.clear();
}

std::shared_ptr<rclcpp::Node> MakeNode(const std::string& name) {
  rclcpp::NodeOptions options;
  options.start_parameter_services(false);
  options.start_parameter_event_publisher(false);
  return std::make_shared<rclcpp::Node>(name, options);
}

constexpr char kRuntimeSnapshot[] =
    R"({"schema_version":2,"scope":"component:thermal_control"})";

class RuntimeLifecycleModeTest : public ::testing::Test {
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

  void SetUp() override {
    ResetFakeState();

    session_ = std::make_unique<RuntimeSession>(FakeApi());
    ASSERT_TRUE(session_->Open(R"({"open":"ok"})").ok());

    server_node_ = MakeNode("runtime_lifecycle_server");
    client_node_ = MakeNode("runtime_lifecycle_client");

    parameter_adapter_ = std::make_unique<RuntimeParameterAdapter>(session_.get());
    control_adapter_ = std::make_unique<RuntimeControlAdapter>(session_.get());

    parameter_node_adapter_ = std::make_unique<RuntimeParameterNodeAdapter>(
        server_node_.get(), parameter_adapter_.get());
    control_node_adapter_ = std::make_unique<RuntimeControlNodeAdapter>(
        server_node_.get(), control_adapter_.get());
    lifecycle_mode_ = std::make_unique<RuntimeLifecycleMode>(
        parameter_node_adapter_.get(), control_node_adapter_.get());

    executor_.add_node(server_node_);
    executor_.add_node(client_node_);
  }

  void TearDown() override {
    lifecycle_mode_.reset();
    control_node_adapter_.reset();
    parameter_node_adapter_.reset();
    control_adapter_.reset();
    parameter_adapter_.reset();

    executor_.remove_node(client_node_);
    executor_.remove_node(server_node_);

    if (session_ != nullptr) {
      EXPECT_EQ(session_->Close(), RuntimeSdkStatus::kOk);
    }
    session_.reset();

    client_node_.reset();
    server_node_.reset();
  }

  template <typename ClientT>
  void WaitForService(const typename rclcpp::Client<ClientT>::SharedPtr& client) {
    const auto deadline = std::chrono::steady_clock::now() +
                          std::chrono::seconds(2);
    while (!client->wait_for_service(std::chrono::milliseconds(50))) {
      executor_.spin_some();
      ASSERT_LT(std::chrono::steady_clock::now(), deadline)
          << "service did not become available";
    }
  }

  template <typename ClientT>
  bool WaitForServiceUnavailable(
      const typename rclcpp::Client<ClientT>::SharedPtr& client) {
    const auto deadline = std::chrono::steady_clock::now() +
                          std::chrono::seconds(1);
    while (client->wait_for_service(std::chrono::milliseconds(50))) {
      executor_.spin_some();
      if (std::chrono::steady_clock::now() >= deadline) {
        return false;
      }
    }
    return true;
  }

  template <typename FutureT>
  void WaitForFuture(FutureT& future) {
    const auto status = executor_.spin_until_future_complete(
        future, std::chrono::seconds(2));
    ASSERT_EQ(status, rclcpp::FutureReturnCode::SUCCESS);
  }

  std::unique_ptr<RuntimeSession> session_;
  std::unique_ptr<RuntimeParameterAdapter> parameter_adapter_;
  std::unique_ptr<RuntimeControlAdapter> control_adapter_;
  std::unique_ptr<RuntimeParameterNodeAdapter> parameter_node_adapter_;
  std::unique_ptr<RuntimeControlNodeAdapter> control_node_adapter_;
  std::unique_ptr<RuntimeLifecycleMode> lifecycle_mode_;
  std::shared_ptr<rclcpp::Node> server_node_;
  std::shared_ptr<rclcpp::Node> client_node_;
  rclcpp::executors::SingleThreadedExecutor executor_;
};

TEST_F(RuntimeLifecycleModeTest, ConfigureActivateDeactivateCleanupFlow) {
  RuntimeLifecycleConfigureRequest configure_request;
  configure_request.scope_root = "component:thermal_control";
  configure_request.service_prefix = "/configflux/lifecycle_control";
  configure_request.callback_actor = "ros2.lifecycle.integration";
  configure_request.callback_reason = "lifecycle active";

  auto configure_result = lifecycle_mode_->OnConfigure(configure_request);
  ASSERT_EQ(configure_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(configure_result.successful) << configure_result.reason;
  EXPECT_EQ(lifecycle_mode_->state(), RuntimeLifecycleState::kInactive);
  EXPECT_FALSE(parameter_node_adapter_->mutations_enabled());
  EXPECT_FALSE(control_node_adapter_->lifecycle_active());

  rclcpp::Parameter max_rpm;
  ASSERT_TRUE(parameter_node_adapter_->GetParameter("thermal_control.max_rpm",
                                                    &max_rpm));
  EXPECT_EQ(max_rpm.as_int(), 4200);

  auto commit_client =
      client_node_->create_client<RuntimeControlNodeAdapter::CommitConfigurationSrv>(
          "/configflux/lifecycle_control/commit_configuration");
  WaitForService<RuntimeControlNodeAdapter::CommitConfigurationSrv>(commit_client);

  auto commit_request =
      std::make_shared<RuntimeControlNodeAdapter::CommitConfigurationSrv::Request>();
  commit_request->runtime_snapshot_json = kRuntimeSnapshot;
  auto commit_future = commit_client->async_send_request(commit_request);
  WaitForFuture(commit_future);
  auto commit_response = commit_future.get();
  EXPECT_FALSE(commit_response->result.successful);
  EXPECT_NE(commit_response->result.reason.find("lifecycle inactive state"),
            std::string::npos);

  auto paused_set_result = server_node_->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{4500})});
  EXPECT_FALSE(paused_set_result.successful);
  EXPECT_NE(paused_set_result.reason.find("lifecycle inactive state"),
            std::string::npos);

  auto activate_result = lifecycle_mode_->OnActivate();
  ASSERT_EQ(activate_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(activate_result.successful) << activate_result.reason;
  EXPECT_EQ(lifecycle_mode_->state(), RuntimeLifecycleState::kActive);
  EXPECT_TRUE(parameter_node_adapter_->mutations_enabled());
  EXPECT_TRUE(control_node_adapter_->lifecycle_active());

  auto active_set_result = server_node_->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{4500})});
  EXPECT_TRUE(active_set_result.successful) << active_set_result.reason;
  EXPECT_NE(g_state.last_atomic_request.find(
                "component.thermal_control.param.max_rpm"),
            std::string::npos);

  commit_future = commit_client->async_send_request(commit_request);
  WaitForFuture(commit_future);
  commit_response = commit_future.get();
  EXPECT_TRUE(commit_response->result.successful)
      << commit_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kCommitConfiguration));

  auto deactivate_result = lifecycle_mode_->OnDeactivate();
  ASSERT_EQ(deactivate_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(deactivate_result.successful) << deactivate_result.reason;
  EXPECT_EQ(lifecycle_mode_->state(), RuntimeLifecycleState::kInactive);

  auto paused_after_deactivate = server_node_->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{4600})});
  EXPECT_FALSE(paused_after_deactivate.successful);
  EXPECT_NE(paused_after_deactivate.reason.find("lifecycle inactive state"),
            std::string::npos);

  commit_future = commit_client->async_send_request(commit_request);
  WaitForFuture(commit_future);
  commit_response = commit_future.get();
  EXPECT_FALSE(commit_response->result.successful);
  EXPECT_NE(commit_response->result.reason.find("lifecycle inactive state"),
            std::string::npos);

  ASSERT_TRUE(parameter_node_adapter_->GetParameter("thermal_control.max_rpm",
                                                    &max_rpm));
  EXPECT_EQ(max_rpm.as_int(), 4500);

  activate_result = lifecycle_mode_->OnActivate();
  ASSERT_TRUE(activate_result.successful) << activate_result.reason;
  ASSERT_TRUE(parameter_node_adapter_->GetParameter("thermal_control.max_rpm",
                                                    &max_rpm));
  EXPECT_EQ(max_rpm.as_int(), 4500);

  auto cleanup_result = lifecycle_mode_->OnCleanup();
  ASSERT_EQ(cleanup_result.status, RuntimeSdkStatus::kOk);
  ASSERT_TRUE(cleanup_result.successful) << cleanup_result.reason;
  EXPECT_EQ(lifecycle_mode_->state(), RuntimeLifecycleState::kUnconfigured);
  EXPECT_TRUE(WaitForServiceUnavailable<RuntimeControlNodeAdapter::CommitConfigurationSrv>(
      commit_client));

  g_state.last_atomic_request.clear();
  auto local_set_after_cleanup = server_node_->set_parameters_atomically(
      {rclcpp::Parameter("thermal_control.max_rpm", int64_t{4700})});
  EXPECT_TRUE(local_set_after_cleanup.successful)
      << local_set_after_cleanup.reason;
  EXPECT_EQ(g_state.last_atomic_request, "");
}

}  // namespace
