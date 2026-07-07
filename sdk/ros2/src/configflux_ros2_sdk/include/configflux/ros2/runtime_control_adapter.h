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

enum class RuntimeControlOutcome : uint8_t {
  kOk = 0,
  kRuntimeError = 1,
  kTransportError = 2,
  kInvalidResponse = 3,
};

enum class PullUpdatesSource : uint8_t {
  kBackend = 0,
  kDirectPush = 1,
};

using PullUpdateValue = std::variant<bool, int64_t, double, std::string>;

struct ControlOperationContext {
  std::string actor;
  std::optional<std::string> reason;
};

struct PullUpdateWrite {
  std::string ros_parameter_name;
  PullUpdateValue value;
  std::optional<std::string> before_leaf_hash;
  std::optional<std::string> after_leaf_hash;
};

struct CommitConfigurationServiceRequest {
  std::string runtime_snapshot_json;
  ControlOperationContext context;
  std::optional<std::string> expected_base_configuration_id;
  std::vector<std::string> changed_ros_parameter_hints;
};

struct RollbackDirtyServiceRequest {
  std::string runtime_snapshot_json;
  ControlOperationContext context;
  std::vector<std::string> rollback_ros_parameter_names;
};

struct CheckForUpdatesServiceRequest {
  std::string runtime_snapshot_json;
  bool backend_connected = true;
  std::optional<std::string> pending_update_summary;
};

struct PullUpdatesActionRequest {
  std::string runtime_snapshot_json;
  ControlOperationContext context;
  bool backend_connected = true;
  PullUpdatesSource source = PullUpdatesSource::kBackend;
  std::vector<PullUpdateWrite> writes;
  std::optional<std::string> base_configuration_id;
  bool full_snapshot = false;
  std::optional<std::string> pending_update_summary;
  std::optional<std::string> target_configuration_id;
};

struct GetSyncStatusServiceRequest {
  std::string runtime_snapshot_json;
};

struct ExportPendingSyncBundleServiceRequest {
  std::string runtime_snapshot_json;
  uint32_t max_audit_events = 256;
};

struct PushAuditEventsServiceRequest {
  std::string runtime_snapshot_json;
  bool backend_connected = true;
  uint32_t max_events = 256;
};

struct RuntimeControlResult {
  sdk::RuntimeSdkStatus status = sdk::RuntimeSdkStatus::kOk;
  RuntimeControlOutcome outcome = RuntimeControlOutcome::kTransportError;
  bool successful = false;
  std::string runtime_status;
  std::string runtime_code;
  std::string response_json;
  std::string reason;
};

class RuntimeControlAdapter {
 public:
  explicit RuntimeControlAdapter(sdk::RuntimeSession* session);

  RuntimeControlResult CommitConfiguration(
      const CommitConfigurationServiceRequest& request) const;
  RuntimeControlResult RollbackDirty(
      const RollbackDirtyServiceRequest& request) const;
  RuntimeControlResult CheckForUpdates(
      const CheckForUpdatesServiceRequest& request) const;
  RuntimeControlResult PullUpdatesAction(
      const PullUpdatesActionRequest& request) const;
  RuntimeControlResult GetSyncStatus(
      const GetSyncStatusServiceRequest& request) const;
  RuntimeControlResult ExportPendingSyncBundle(
      const ExportPendingSyncBundleServiceRequest& request) const;
  RuntimeControlResult PushAuditEvents(
      const PushAuditEventsServiceRequest& request) const;

 private:
  using RuntimeCall = sdk::RuntimeCallResult (sdk::RuntimeSession::*)(
      std::string_view request_json);

  RuntimeControlResult ExecuteMappedCall(std::string request_json, RuntimeCall call,
                                         std::string_view operation_name) const;

  static bool ExtractJsonStringField(std::string_view json,
                                     std::string_view field_name, std::string* out);
  static bool ExtractRuntimeCode(std::string_view response_json,
                                 std::string* out_code);
  static std::string PullUpdateValueToJson(const PullUpdateValue& value);
  static std::string PullUpdatesSourceToJson(PullUpdatesSource source);
  static std::string EscapeJsonString(std::string_view value);

  sdk::RuntimeSession* session_ = nullptr;
};

}  // namespace configflux::ros2
