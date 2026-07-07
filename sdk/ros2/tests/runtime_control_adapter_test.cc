// SPDX-License-Identifier: BUSL-1.1

#include "configflux/ros2/runtime_control_adapter.h"

#include <cstdlib>
#include <cstring>
#include <iostream>
#include <limits>
#include <string>
#include <vector>

namespace {

using configflux::ros2::CheckForUpdatesServiceRequest;
using configflux::ros2::CommitConfigurationServiceRequest;
using configflux::ros2::ControlOperationContext;
using configflux::ros2::ExportPendingSyncBundleServiceRequest;
using configflux::ros2::GetSyncStatusServiceRequest;
using configflux::ros2::PullUpdatesActionRequest;
using configflux::ros2::PullUpdatesSource;
using configflux::ros2::PullUpdateWrite;
using configflux::ros2::PushAuditEventsServiceRequest;
using configflux::ros2::RollbackDirtyServiceRequest;
using configflux::ros2::RuntimeControlAdapter;
using configflux::ros2::RuntimeControlOutcome;
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
  g_state.last_operation = operation;
  g_state.last_request_json = request;

  std::string response;
  if (operation == static_cast<uint32_t>(RuntimeOperation::kCommitConfiguration)) {
    if (request.find(R"("actor":"reject.commit")") != std::string::npos) {
      response =
          R"({"status":"error","diagnostics":{"diagnostics":[{"code":"E_RUNTIME_SYNC_BASE_MISMATCH"}]}})";
    } else {
      response = R"({"status":"ok","commit_id":"commit-1"})";
    }
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kRollbackDirty)) {
    response =
        R"({"status":"ok","rolled_back_paths":["component.thermal_control.param.max_rpm"]})";
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kCheckForUpdates)) {
    response = R"({"status":"ok","sync_status":{"sync_state":"offline"}})";
  } else if (operation == static_cast<uint32_t>(RuntimeOperation::kPullUpdates)) {
    if (request.find("missing_status") != std::string::npos) {
      response = R"({"applied_paths":[]})";
    } else {
      response =
          R"({"status":"ok","applied_paths":["component.thermal_control.param.max_rpm"]})";
    }
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kGetSyncStatus)) {
    response = R"({"status":"ok","sync_status":{"sync_state":"idle"}})";
  } else if (operation ==
             static_cast<uint32_t>(RuntimeOperation::kExportPendingSyncBundle)) {
    response = R"({"status":"ok","bundle":{"bundle_id":"offline-sync-1"}})";
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

#define CHECK_TRUE(expr)                                                        \
  do {                                                                          \
    if (!(expr)) {                                                              \
      std::cerr << "check failed at " << __FILE__ << ":" << __LINE__ << ": "   \
                << #expr << std::endl;                                          \
      return false;                                                             \
    }                                                                           \
  } while (false)

#define CHECK_EQ(lhs, rhs) CHECK_TRUE((lhs) == (rhs))

constexpr char kRuntimeSnapshot[] =
    R"({"schema_version":2,"scope":"component:thermal_control"})";

bool TestCommitConfigurationMapsRuntimeV2Request() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  CommitConfigurationServiceRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.context = ControlOperationContext{
      .actor = "ros2.commit_service",
      .reason = std::optional<std::string>("operator commit"),
  };
  request.expected_base_configuration_id = "abc123";
  request.changed_ros_parameter_hints = {
      "thermal_control.max_rpm",
      "component.thermal_control.param.control_driver",
  };

  const auto result = adapter.CommitConfiguration(request);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(result.successful);
  CHECK_EQ(g_state.last_operation,
           static_cast<uint32_t>(RuntimeOperation::kCommitConfiguration));
  CHECK_TRUE(g_state.last_request_json.find(R"("actor":"ros2.commit_service")") !=
             std::string::npos);
  CHECK_TRUE(g_state.last_request_json.find(R"("expected_base_configuration_id":"abc123")") !=
             std::string::npos);
  CHECK_TRUE(g_state.last_request_json.find(
                 R"("changed_paths_hint":["component.thermal_control.param.max_rpm","component.thermal_control.param.control_driver"])") !=
             std::string::npos);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestRollbackDirtySubsetConvertsRosNames() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  RollbackDirtyServiceRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.context.actor = "ros2.rollback_service";
  request.rollback_ros_parameter_names = {"thermal_control.max_rpm"};

  const auto result = adapter.RollbackDirty(request);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(result.successful);
  CHECK_EQ(g_state.last_operation,
           static_cast<uint32_t>(RuntimeOperation::kRollbackDirty));
  CHECK_TRUE(g_state.last_request_json.find(R"("mode":"subset")") !=
             std::string::npos);
  CHECK_TRUE(g_state.last_request_json.find(
                 R"("paths":["component.thermal_control.param.max_rpm"])") !=
             std::string::npos);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestCheckForUpdatesAndGetSyncStatusMapServices() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  CheckForUpdatesServiceRequest check_request;
  check_request.runtime_snapshot_json = kRuntimeSnapshot;
  check_request.backend_connected = false;
  check_request.pending_update_summary = "one update available";

  auto check_result = adapter.CheckForUpdates(check_request);
  CHECK_EQ(check_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(check_result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(check_result.successful);
  CHECK_EQ(g_state.last_operation,
           static_cast<uint32_t>(RuntimeOperation::kCheckForUpdates));
  CHECK_TRUE(g_state.last_request_json.find(R"("backend_connected":false)") !=
             std::string::npos);

  GetSyncStatusServiceRequest status_request;
  status_request.runtime_snapshot_json = kRuntimeSnapshot;
  auto status_result = adapter.GetSyncStatus(status_request);
  CHECK_EQ(status_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(status_result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(status_result.successful);
  CHECK_EQ(g_state.last_operation,
           static_cast<uint32_t>(RuntimeOperation::kGetSyncStatus));

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestPullUpdatesActionSupportsOfflineReconcileShape() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  PullUpdatesActionRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.context.reason = "apply offline bundle";
  request.backend_connected = false;
  request.source = PullUpdatesSource::kDirectPush;
  request.base_configuration_id =
      "base-config-id-11111111111111111111111111111111";
  request.target_configuration_id =
      "target-config-id-222222222222222222222222222222";
  request.pending_update_summary = "offline direct push";
  request.writes = {
      PullUpdateWrite{
          .ros_parameter_name = "thermal_control.max_rpm",
          .value = 4700LL,
          .before_leaf_hash = std::optional<std::string>("aabb"),
          .after_leaf_hash = std::optional<std::string>("ccdd"),
      },
  };

  const auto result = adapter.PullUpdatesAction(request);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(result.successful);
  CHECK_EQ(g_state.last_operation,
           static_cast<uint32_t>(RuntimeOperation::kPullUpdates));
  CHECK_TRUE(g_state.last_request_json.find(R"("actor":"ros2.reconcile_offline")") !=
             std::string::npos);
  CHECK_TRUE(g_state.last_request_json.find(R"("source":"direct_push")") !=
             std::string::npos);
  CHECK_TRUE(g_state.last_request_json.find(
                 R"("path":"component.thermal_control.param.max_rpm")") !=
             std::string::npos);
  CHECK_TRUE(g_state.last_request_json.find(R"("backend_connected":false)") !=
             std::string::npos);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestControlCharactersAreEscapedInRequestPayloads() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);

  std::string actor = "ops";
  actor.push_back('\x01');
  actor.append("controller");
  std::string reason = "line";
  reason.push_back('\x1f');
  reason.append("separator");

  CommitConfigurationServiceRequest commit_request;
  commit_request.runtime_snapshot_json = kRuntimeSnapshot;
  commit_request.context.actor = actor;
  commit_request.context.reason = reason;

  const auto commit_result = adapter.CommitConfiguration(commit_request);
  CHECK_EQ(commit_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(commit_result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(commit_result.successful);
  CHECK_TRUE(
      g_state.last_request_json.find(R"("actor":"ops\u0001controller")") !=
      std::string::npos);
  CHECK_TRUE(
      g_state.last_request_json.find(R"("reason":"line\u001fseparator")") !=
      std::string::npos);

  std::string control_value = "auto";
  control_value.push_back('\x1e');
  control_value.append("mode");

  PullUpdatesActionRequest pull_request;
  pull_request.runtime_snapshot_json = kRuntimeSnapshot;
  pull_request.writes = {
      PullUpdateWrite{
          .ros_parameter_name = "thermal_control.control_driver",
          .value = control_value,
      },
  };

  const auto pull_result = adapter.PullUpdatesAction(pull_request);
  CHECK_EQ(pull_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(pull_result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(pull_result.successful);
  CHECK_TRUE(
      g_state.last_request_json.find(R"("value":"auto\u001emode")") !=
      std::string::npos);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestRuntimeErrorClassificationExtractsRuntimeCode() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  CommitConfigurationServiceRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.context.actor = "reject.commit";

  const auto result = adapter.CommitConfiguration(request);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(result.outcome, RuntimeControlOutcome::kRuntimeError);
  CHECK_TRUE(!result.successful);
  CHECK_EQ(result.runtime_code, "E_RUNTIME_SYNC_BASE_MISMATCH");

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestMalformedRuntimePrefixPathHintIsRejected() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  CommitConfigurationServiceRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.changed_ros_parameter_hints = {"component.only_prefix"};

  const auto result = adapter.CommitConfiguration(request);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kInvalidArgument);
  CHECK_EQ(result.outcome, RuntimeControlOutcome::kTransportError);
  CHECK_TRUE(!result.successful);
  CHECK_TRUE(result.reason.find("invalid changed path hint") != std::string::npos);
  CHECK_EQ(g_state.last_operation, static_cast<uint32_t>(0));

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestInvalidResponseClassificationIsDeterministic() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  PullUpdatesActionRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.context.actor = "missing_status";
  request.source = PullUpdatesSource::kBackend;

  const auto result = adapter.PullUpdatesAction(request);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kInvalidJson);
  CHECK_EQ(result.outcome, RuntimeControlOutcome::kInvalidResponse);
  CHECK_TRUE(!result.successful);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestPullUpdatesRejectsNonFiniteDoubles() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  PullUpdatesActionRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.writes = {
      PullUpdateWrite{
          .ros_parameter_name = "thermal_control.max_rpm",
          .value = std::numeric_limits<double>::quiet_NaN(),
      },
  };

  const auto nan_result = adapter.PullUpdatesAction(request);
  CHECK_EQ(nan_result.status, configflux::sdk::RuntimeSdkStatus::kInvalidArgument);
  CHECK_EQ(nan_result.outcome, RuntimeControlOutcome::kTransportError);
  CHECK_TRUE(!nan_result.successful);
  CHECK_TRUE(nan_result.reason.find("non-finite double") != std::string::npos);
  CHECK_EQ(g_state.last_operation, static_cast<uint32_t>(0));

  request.writes[0].value = std::numeric_limits<double>::infinity();
  const auto inf_result = adapter.PullUpdatesAction(request);
  CHECK_EQ(inf_result.status, configflux::sdk::RuntimeSdkStatus::kInvalidArgument);
  CHECK_EQ(inf_result.outcome, RuntimeControlOutcome::kTransportError);
  CHECK_TRUE(!inf_result.successful);
  CHECK_TRUE(inf_result.reason.find("non-finite double") != std::string::npos);
  CHECK_EQ(g_state.last_operation, static_cast<uint32_t>(0));

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestPullUpdatesRejectsUnknownSource() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);
  PullUpdatesActionRequest request;
  request.runtime_snapshot_json = kRuntimeSnapshot;
  request.source = static_cast<PullUpdatesSource>(255);

  const auto result = adapter.PullUpdatesAction(request);
  CHECK_EQ(result.status, configflux::sdk::RuntimeSdkStatus::kInvalidArgument);
  CHECK_EQ(result.outcome, RuntimeControlOutcome::kTransportError);
  CHECK_TRUE(!result.successful);
  CHECK_TRUE(result.reason.find("unknown source") != std::string::npos);
  CHECK_EQ(g_state.last_operation, static_cast<uint32_t>(0));

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

bool TestExportAndPushAuditMapOfflineWorkflows() {
  ResetFakeState();
  RuntimeSession session(FakeApi());
  CHECK_TRUE(session.Open(R"({"open":"ok"})").ok());

  RuntimeControlAdapter adapter(&session);

  ExportPendingSyncBundleServiceRequest export_request;
  export_request.runtime_snapshot_json = kRuntimeSnapshot;
  export_request.max_audit_events = 64;
  const auto export_result = adapter.ExportPendingSyncBundle(export_request);
  CHECK_EQ(export_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(export_result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(export_result.successful);
  CHECK_EQ(g_state.last_operation,
           static_cast<uint32_t>(RuntimeOperation::kExportPendingSyncBundle));
  CHECK_TRUE(g_state.last_request_json.find(R"("max_audit_events":64)") !=
             std::string::npos);

  PushAuditEventsServiceRequest push_request;
  push_request.runtime_snapshot_json = kRuntimeSnapshot;
  push_request.backend_connected = true;
  push_request.max_events = 12;
  const auto push_result = adapter.PushAuditEvents(push_request);
  CHECK_EQ(push_result.status, configflux::sdk::RuntimeSdkStatus::kOk);
  CHECK_EQ(push_result.outcome, RuntimeControlOutcome::kOk);
  CHECK_TRUE(push_result.successful);
  CHECK_EQ(g_state.last_operation,
           static_cast<uint32_t>(RuntimeOperation::kPushAuditEvents));
  CHECK_TRUE(g_state.last_request_json.find(R"("max_events":12)") !=
             std::string::npos);

  CHECK_EQ(session.Close(), configflux::sdk::RuntimeSdkStatus::kOk);
  return true;
}

}  // namespace

int main() {
  bool ok = true;
  ok = TestCommitConfigurationMapsRuntimeV2Request() && ok;
  ok = TestRollbackDirtySubsetConvertsRosNames() && ok;
  ok = TestCheckForUpdatesAndGetSyncStatusMapServices() && ok;
  ok = TestPullUpdatesActionSupportsOfflineReconcileShape() && ok;
  ok = TestControlCharactersAreEscapedInRequestPayloads() && ok;
  ok = TestRuntimeErrorClassificationExtractsRuntimeCode() && ok;
  ok = TestMalformedRuntimePrefixPathHintIsRejected() && ok;
  ok = TestInvalidResponseClassificationIsDeterministic() && ok;
  ok = TestPullUpdatesRejectsNonFiniteDoubles() && ok;
  ok = TestPullUpdatesRejectsUnknownSource() && ok;
  ok = TestExportAndPushAuditMapOfflineWorkflows() && ok;

  if (!ok) {
    std::cerr << "runtime_control_adapter_test failed" << std::endl;
    return 1;
  }
  std::cout << "runtime_control_adapter_test passed" << std::endl;
  return 0;
}
