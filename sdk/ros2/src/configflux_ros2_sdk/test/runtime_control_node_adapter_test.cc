// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_control_adapter.h"
#include "configflux/ros2/runtime_control_node_adapter.h"

#include <chrono>
#include <cstdlib>
#include <cstring>
#include <memory>
#include <string>

#include "gtest/gtest.h"
#include "rclcpp/rclcpp.hpp"
#include "rclcpp_action/rclcpp_action.hpp"

namespace {

using configflux::ros2::RuntimeControlAdapter;
using configflux::ros2::RuntimeControlNodeAdapter;
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
  if (operation == static_cast<uint32_t>(RuntimeOperation::kCommitConfiguration)) {
    response = R"({"status":"ok","commit_id":"commit-1"})";
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kRollbackDirty)) {
    response = R"({"status":"ok","rolled_back_paths":["component.thermal_control.param.max_rpm"]})";
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kCheckForUpdates)) {
    response = R"({"status":"ok","sync_status":{"sync_state":"offline"}})";
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kPullUpdates)) {
    response = R"({"status":"ok","applied_paths":["component.thermal_control.param.max_rpm"]})";
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kGetSyncStatus)) {
    response = R"({"status":"ok","sync_status":{"sync_state":"idle"}})";
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kExportPendingSyncBundle)) {
    response = R"({"status":"ok","bundle":{"bundle_id":"offline-1"}})";
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kPushAuditEvents)) {
    response = R"({"status":"ok","pushed_count":2,"pending_count":0})";
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
}

std::shared_ptr<rclcpp::Node> MakeNode(const std::string& name) {
  rclcpp::NodeOptions options;
  options.start_parameter_services(false);
  options.start_parameter_event_publisher(false);
  return std::make_shared<rclcpp::Node>(name, options);
}

constexpr char kRuntimeSnapshot[] =
    R"({"schema_version":2,"scope":"component:thermal_control"})";

class RuntimeControlNodeAdapterTest : public ::testing::Test {
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

    server_node_ = MakeNode("runtime_control_server");
    client_node_ = MakeNode("runtime_control_client");

    runtime_adapter_ = std::make_unique<RuntimeControlAdapter>(session_.get());
    node_adapter_ = std::make_unique<RuntimeControlNodeAdapter>(
        server_node_.get(), runtime_adapter_.get());

    ASSERT_TRUE(node_adapter_->StartServers("/configflux/runtime_control"));

    executor_.add_node(server_node_);
    executor_.add_node(client_node_);
  }

  void TearDown() override {
    executor_.remove_node(client_node_);
    executor_.remove_node(server_node_);

    node_adapter_.reset();
    runtime_adapter_.reset();

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

  template <typename FutureT>
  void WaitForFuture(FutureT& future) {
    const auto status = executor_.spin_until_future_complete(
        future, std::chrono::seconds(2));
    ASSERT_EQ(status, rclcpp::FutureReturnCode::SUCCESS);
  }

  std::unique_ptr<RuntimeSession> session_;
  std::unique_ptr<RuntimeControlAdapter> runtime_adapter_;
  std::unique_ptr<RuntimeControlNodeAdapter> node_adapter_;
  std::shared_ptr<rclcpp::Node> server_node_;
  std::shared_ptr<rclcpp::Node> client_node_;
  rclcpp::executors::SingleThreadedExecutor executor_;
};

TEST_F(RuntimeControlNodeAdapterTest, ServicesMapToRuntimeControlOperations) {
  auto commit_client =
      client_node_->create_client<RuntimeControlNodeAdapter::CommitConfigurationSrv>(
          "/configflux/runtime_control/commit_configuration");
  WaitForService<RuntimeControlNodeAdapter::CommitConfigurationSrv>(commit_client);

  auto commit_request =
      std::make_shared<RuntimeControlNodeAdapter::CommitConfigurationSrv::Request>();
  commit_request->runtime_snapshot_json = kRuntimeSnapshot;
  commit_request->actor = "ros2.commit_service";
  commit_request->reason = "operator commit";
  commit_request->expected_base_configuration_id = "base-id";
  commit_request->changed_ros_parameter_hints = {"thermal_control.max_rpm"};

  auto commit_future = commit_client->async_send_request(commit_request);
  WaitForFuture(commit_future);
  auto commit_response = commit_future.get();
  ASSERT_TRUE(commit_response->result.successful)
      << commit_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kCommitConfiguration));
  EXPECT_NE(g_state.last_request_json.find(R"("actor":"ros2.commit_service")"),
            std::string::npos);

  auto rollback_client =
      client_node_->create_client<RuntimeControlNodeAdapter::RollbackDirtySrv>(
          "/configflux/runtime_control/rollback_dirty");
  WaitForService<RuntimeControlNodeAdapter::RollbackDirtySrv>(rollback_client);

  auto rollback_request =
      std::make_shared<RuntimeControlNodeAdapter::RollbackDirtySrv::Request>();
  rollback_request->runtime_snapshot_json = kRuntimeSnapshot;
  rollback_request->actor = "ros2.rollback_service";
  rollback_request->rollback_ros_parameter_names = {"thermal_control.max_rpm"};

  auto rollback_future = rollback_client->async_send_request(rollback_request);
  WaitForFuture(rollback_future);
  auto rollback_response = rollback_future.get();
  ASSERT_TRUE(rollback_response->result.successful)
      << rollback_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kRollbackDirty));

  auto check_client =
      client_node_->create_client<RuntimeControlNodeAdapter::CheckForUpdatesSrv>(
          "/configflux/runtime_control/check_for_updates");
  WaitForService<RuntimeControlNodeAdapter::CheckForUpdatesSrv>(check_client);

  auto check_request =
      std::make_shared<RuntimeControlNodeAdapter::CheckForUpdatesSrv::Request>();
  check_request->runtime_snapshot_json = kRuntimeSnapshot;
  check_request->backend_connected = false;
  check_request->pending_update_summary = "offline";

  auto check_future = check_client->async_send_request(check_request);
  WaitForFuture(check_future);
  auto check_response = check_future.get();
  ASSERT_TRUE(check_response->result.successful)
      << check_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kCheckForUpdates));

  auto sync_status_client =
      client_node_->create_client<RuntimeControlNodeAdapter::GetSyncStatusSrv>(
          "/configflux/runtime_control/get_sync_status");
  WaitForService<RuntimeControlNodeAdapter::GetSyncStatusSrv>(sync_status_client);

  auto sync_status_request =
      std::make_shared<RuntimeControlNodeAdapter::GetSyncStatusSrv::Request>();
  sync_status_request->runtime_snapshot_json = kRuntimeSnapshot;

  auto sync_status_future =
      sync_status_client->async_send_request(sync_status_request);
  WaitForFuture(sync_status_future);
  auto sync_status_response = sync_status_future.get();
  ASSERT_TRUE(sync_status_response->result.successful)
      << sync_status_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kGetSyncStatus));

  auto export_client = client_node_
                           ->create_client<RuntimeControlNodeAdapter::
                                               ExportPendingSyncBundleSrv>(
                               "/configflux/runtime_control/"
                               "export_pending_sync_bundle");
  WaitForService<RuntimeControlNodeAdapter::ExportPendingSyncBundleSrv>(
      export_client);

  auto export_request =
      std::make_shared<RuntimeControlNodeAdapter::ExportPendingSyncBundleSrv::
                           Request>();
  export_request->runtime_snapshot_json = kRuntimeSnapshot;
  export_request->max_audit_events = 64;

  auto export_future = export_client->async_send_request(export_request);
  WaitForFuture(export_future);
  auto export_response = export_future.get();
  ASSERT_TRUE(export_response->result.successful)
      << export_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kExportPendingSyncBundle));

  auto push_client =
      client_node_->create_client<RuntimeControlNodeAdapter::PushAuditEventsSrv>(
          "/configflux/runtime_control/push_audit_events");
  WaitForService<RuntimeControlNodeAdapter::PushAuditEventsSrv>(push_client);

  auto push_request =
      std::make_shared<RuntimeControlNodeAdapter::PushAuditEventsSrv::Request>();
  push_request->runtime_snapshot_json = kRuntimeSnapshot;
  push_request->backend_connected = true;
  push_request->max_events = 11;

  auto push_future = push_client->async_send_request(push_request);
  WaitForFuture(push_future);
  auto push_response = push_future.get();
  ASSERT_TRUE(push_response->result.successful) << push_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kPushAuditEvents));
}

TEST_F(RuntimeControlNodeAdapterTest, PullActionMapsReconcileFlow) {
  using PullAction = RuntimeControlNodeAdapter::PullUpdatesAction;

  auto action_client = rclcpp_action::create_client<PullAction>(
      client_node_, "/configflux/runtime_control/pull_updates");
  ASSERT_TRUE(action_client->wait_for_action_server(std::chrono::seconds(2)));

  PullAction::Goal goal;
  goal.runtime_snapshot_json = kRuntimeSnapshot;
  goal.reason = "apply offline bundle";
  goal.backend_connected = false;
  goal.source = PullAction::Goal::SOURCE_DIRECT_PUSH;
  goal.base_configuration_id =
      "base-config-id-11111111111111111111111111111111";
  goal.target_configuration_id =
      "target-config-id-222222222222222222222222222222";
  goal.pending_update_summary = "offline direct push";

  configflux_ros2_sdk::msg::PullUpdateWrite write;
  write.ros_parameter_name = "thermal_control.max_rpm";
  write.value_type =
      configflux_ros2_sdk::msg::PullUpdateWrite::VALUE_TYPE_INT64;
  write.int_value = 4700;
  write.before_leaf_hash = "aabb";
  write.after_leaf_hash = "ccdd";
  goal.writes.push_back(write);

  auto send_goal_future = action_client->async_send_goal(goal);
  WaitForFuture(send_goal_future);
  auto goal_handle = send_goal_future.get();
  ASSERT_NE(goal_handle, nullptr);

  auto result_future = action_client->async_get_result(goal_handle);
  WaitForFuture(result_future);
  const auto wrapped_result = result_future.get();

  ASSERT_NE(wrapped_result.result, nullptr);
  EXPECT_EQ(wrapped_result.code, rclcpp_action::ResultCode::SUCCEEDED);
  EXPECT_TRUE(wrapped_result.result->result.successful)
      << wrapped_result.result->result.reason;

  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kPullUpdates));
  EXPECT_NE(g_state.last_request_json.find(R"("source":"direct_push")"),
            std::string::npos);
  EXPECT_NE(g_state.last_request_json.find(
                R"("path":"component.thermal_control.param.max_rpm")"),
            std::string::npos);
  EXPECT_NE(g_state.last_request_json.find(R"("backend_connected":false)"),
            std::string::npos);
}

TEST_F(RuntimeControlNodeAdapterTest, PullActionRejectsUnknownSourceValue) {
  using PullAction = RuntimeControlNodeAdapter::PullUpdatesAction;

  auto action_client = rclcpp_action::create_client<PullAction>(
      client_node_, "/configflux/runtime_control/pull_updates");
  ASSERT_TRUE(action_client->wait_for_action_server(std::chrono::seconds(2)));

  PullAction::Goal goal;
  goal.runtime_snapshot_json = kRuntimeSnapshot;
  goal.source = 255;

  auto send_goal_future = action_client->async_send_goal(goal);
  WaitForFuture(send_goal_future);
  auto goal_handle = send_goal_future.get();
  ASSERT_NE(goal_handle, nullptr);

  auto result_future = action_client->async_get_result(goal_handle);
  WaitForFuture(result_future);
  const auto wrapped_result = result_future.get();

  ASSERT_NE(wrapped_result.result, nullptr);
  EXPECT_EQ(wrapped_result.code, rclcpp_action::ResultCode::ABORTED);
  EXPECT_FALSE(wrapped_result.result->result.successful);
  EXPECT_EQ(wrapped_result.result->result.sdk_status,
            static_cast<uint32_t>(RuntimeSdkStatus::kInvalidArgument));
  EXPECT_NE(wrapped_result.result->result.reason.find("unknown source"),
            std::string::npos);
  EXPECT_EQ(g_state.last_operation, 0U);
}

TEST_F(RuntimeControlNodeAdapterTest,
       InactiveLifecycleModePausesMutatingAndSyncSurfaces) {
  node_adapter_->SetLifecycleActive(false);

  auto commit_client =
      client_node_->create_client<RuntimeControlNodeAdapter::CommitConfigurationSrv>(
          "/configflux/runtime_control/commit_configuration");
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
  EXPECT_EQ(g_state.last_operation, 0U);

  auto check_client =
      client_node_->create_client<RuntimeControlNodeAdapter::CheckForUpdatesSrv>(
          "/configflux/runtime_control/check_for_updates");
  WaitForService<RuntimeControlNodeAdapter::CheckForUpdatesSrv>(check_client);

  auto check_request =
      std::make_shared<RuntimeControlNodeAdapter::CheckForUpdatesSrv::Request>();
  check_request->runtime_snapshot_json = kRuntimeSnapshot;
  auto check_future = check_client->async_send_request(check_request);
  WaitForFuture(check_future);
  auto check_response = check_future.get();
  EXPECT_FALSE(check_response->result.successful);
  EXPECT_NE(check_response->result.reason.find("lifecycle inactive state"),
            std::string::npos);
  EXPECT_EQ(g_state.last_operation, 0U);

  auto sync_status_client =
      client_node_->create_client<RuntimeControlNodeAdapter::GetSyncStatusSrv>(
          "/configflux/runtime_control/get_sync_status");
  WaitForService<RuntimeControlNodeAdapter::GetSyncStatusSrv>(sync_status_client);

  auto sync_status_request =
      std::make_shared<RuntimeControlNodeAdapter::GetSyncStatusSrv::Request>();
  sync_status_request->runtime_snapshot_json = kRuntimeSnapshot;
  auto sync_status_future =
      sync_status_client->async_send_request(sync_status_request);
  WaitForFuture(sync_status_future);
  auto sync_status_response = sync_status_future.get();
  EXPECT_TRUE(sync_status_response->result.successful)
      << sync_status_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kGetSyncStatus));

  using PullAction = RuntimeControlNodeAdapter::PullUpdatesAction;
  auto action_client = rclcpp_action::create_client<PullAction>(
      client_node_, "/configflux/runtime_control/pull_updates");
  ASSERT_TRUE(action_client->wait_for_action_server(std::chrono::seconds(2)));

  PullAction::Goal goal;
  goal.runtime_snapshot_json = kRuntimeSnapshot;
  auto send_goal_future = action_client->async_send_goal(goal);
  WaitForFuture(send_goal_future);
  EXPECT_EQ(send_goal_future.get(), nullptr);
}

TEST_F(RuntimeControlNodeAdapterTest, LifecyclePolicyCanAllowInactiveCommit) {
  configflux::ros2::RuntimeControlLifecyclePolicy policy;
  policy.allow_commit_configuration_while_inactive = true;
  node_adapter_->SetLifecyclePolicy(policy);
  node_adapter_->SetLifecycleActive(false);

  auto commit_client =
      client_node_->create_client<RuntimeControlNodeAdapter::CommitConfigurationSrv>(
          "/configflux/runtime_control/commit_configuration");
  WaitForService<RuntimeControlNodeAdapter::CommitConfigurationSrv>(commit_client);

  auto commit_request =
      std::make_shared<RuntimeControlNodeAdapter::CommitConfigurationSrv::Request>();
  commit_request->runtime_snapshot_json = kRuntimeSnapshot;
  auto commit_future = commit_client->async_send_request(commit_request);
  WaitForFuture(commit_future);
  auto commit_response = commit_future.get();
  EXPECT_TRUE(commit_response->result.successful)
      << commit_response->result.reason;
  EXPECT_EQ(g_state.last_operation,
            static_cast<uint32_t>(RuntimeOperation::kCommitConfiguration));
}

}  // namespace
